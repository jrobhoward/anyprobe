# anyprobe

USDT-style probes for Rust. A probe is a named point in a program that a
tracer attaches to while the program runs. `anyprobe` emits each platform's
native probe format, so the platform's own tools attach to a running process
with no restart and no instrumentation library loaded into it. With no tracer
attached, a probe costs one enabled check and its arguments are not computed.

| Target | Probe format | Tracers |
|---|---|---|
| Linux, x86-64 and AArch64 | SystemTap SDT notes with semaphores | bpftrace, perf |
| macOS, x86-64 and AArch64 | DTrace USDT, built by the linker | dtrace |
| FreeBSD, x86-64 | DTrace USDT, registered at startup | dtrace |
| Windows | ETW TraceLogging | WPR, PerfView, logman |
| anything else | none; probes compile to nothing | none |

Status: pre-release.

## Example

`#[anyprobe::probe]` probes a function's entry and return:

```rust
#[derive(Debug)]
struct Options {
    verbose: bool,
}

#[anyprobe::probe(provider = "myapp", debug(opts), ret = native)]
fn handle(id: u64, path: &str, opts: &Options) -> u32 {
    // ...
    0
}
```

This defines the probes `myapp:handle__entry(id, path, opts)` and
`myapp:handle__return(ret)`. Integers, `bool`, `char`, raw pointers, `&str`,
`&[u8]`, their `Option`s and `&CStr` are passed as they are. Other arguments
are listed in `debug(..)` (encoded with `{:?}`), `serde(..)` (encoded as
JSON) or `skip(..)`; an unlisted one is a compile error that names these fixes. The
`autoref` feature picks an encoding for unlisted arguments instead. Encoding
runs only while a tracer is attached.

On an `async fn` the entry probe fires when the body starts and the return
probe when it completes, and both pass an invocation id first, so a tracer
can pair them when calls interleave. Two options add to what is traced:

```rust
#[anyprobe::probe(provider = "myapp", unwind)]
async fn fetch(id: u64) -> u64 {
    // ...
    id
}

#[anyprobe::probe(provider = "myapp", symbol)]
fn checksum(data: &[u8]) -> u32 {
    // ...
    0
}
```

`unwind` adds `fetch__unwind(invocation, panicking)`, which fires when a
panic unwinds through the function or, for an `async fn`, when its future is
dropped before completing. `symbol` exports `checksum` as `myapp__checksum`,
never inlined, for tools that attach by symbol: raw uprobes, DTrace's `pid`
provider.

`probes!` defines probes to fire from anywhere in a function:

```rust
anyprobe::probes! {
    provider = "myapp";

    /// A request started: its id and path.
    pub fn request__start(id: u64, path: &str);
}

fn handle(id: u64, path: &str) {
    anyprobe::fire!(request__start(id, path));
    // ...
}
```

Each probe becomes a module with `enabled()` and `fire(...)`.
`anyprobe::fire!` calls `fire` only when `enabled()` is true, so its
arguments are computed only while a tracer is attached. Calling `enabled()`
directly suits one check that guards several probes or other work; `fire`
on its own evaluates its arguments every time. `probes!` takes
the native types only, up to five values per probe (`&str`, `&[u8]` and
their `Option`s count as two: pointer and length; `None` is a null pointer).

`anyprobe::next_id()` returns an id that is unique in the process and never
0. Passing it to related probes lets a tracer pair them when calls
interleave:

```rust
anyprobe::probes! {
    provider = "myapp";

    pub fn job__start(id: u64, name: &str);
    pub fn job__done(id: u64, ok: bool);
}

fn run(name: &str) {
    let id = if job__start::enabled() {
        let id = anyprobe::next_id();
        job__start::fire(id, name);
        id
    } else {
        0
    };
    // ...
    if job__done::enabled() {
        job__done::fire(id, true);
    }
}
```

An id of 0 means the first probe was off when the call started.

The provider defaults to the crate name. DTrace does not allow one that ends
in a digit, so such a crate name gets a `_` after it: the probes of a crate
named `http2` have the provider `http2_` unless it sets `provider = "..."`.

## Attaching

Linux, with bpftrace (`str(ptr, len)` reads a `&str`):

```text
sudo bpftrace -p PID -e 'usdt:/path/to/binary:myapp:request__start { printf("%d %s\n", arg0, str(arg1, arg2)); }'
```

macOS and FreeBSD, with dtrace (DTrace shows `__` in a probe name as `-`):

```text
sudo dtrace -p PID -n 'myapp$target:::request-start { printf("%d %s\n", arg0, copyinstr(arg1, arg2)); }'
```

Windows: the provider is named after the probe provider (`myapp`), with the
standard TraceLogging GUID derived from that name. PerfView accepts `*myapp`;
`logman` and `tracelog` need the GUID. The provider registers with ETW the
first time any of its probes is checked.

The walkthroughs in [docs/usage](https://github.com/jrobhoward/anyprobe/tree/main/docs/usage)
run the `demo` example in one terminal and a tracer in another, with the
output to expect: [Linux](https://github.com/jrobhoward/anyprobe/blob/main/docs/usage/linux.md),
[macOS](https://github.com/jrobhoward/anyprobe/blob/main/docs/usage/macos.md),
[FreeBSD](https://github.com/jrobhoward/anyprobe/blob/main/docs/usage/freebsd.md),
[Windows](https://github.com/jrobhoward/anyprobe/blob/main/docs/usage/windows.md).

Select probes by provider and probe name. DTrace also reports a function: on
macOS the mangled Rust symbol that holds the site, which changes between
builds; on FreeBSD the name of the function `#[probe]` annotates, or nothing
for `probes!`.

An encoded argument (`debug`, `serde`, or arguments combined into one JSON
object) is a string: a pointer and a length, followed by a NUL byte. bpftrace
reads it with `str(ptr, len)`, dtrace with `copyinstr(ptr, len)`; perf and
gdb read it as a NUL-terminated string. Encoded values are cut at 4096
bytes, and a cut value ends with `...`; bpftrace reads 64 bytes by default
(`BPFTRACE_MAX_STRLEN`). A native
`&str` has no NUL after it, so perf and gdb do not read one correctly.

## Listing probes

Each probe is also described in a section of the binary: provider, name,
arguments and their types, and the file and line that define it.
`anyprobe::list()` reads the description from the running program.
`cargo anyprobe` reads it from a built binary for any target, without running
it, and writes tracer scripts that print every probe with its arguments
decoded:

```text
cargo install cargo-anyprobe
cargo anyprobe list --bin myapp --release
cargo anyprobe bpftrace target/release/myapp > myapp.bt      # sudo bpftrace -p PID myapp.bt
cargo anyprobe dtrace --bin myapp --release > myapp.d        # sudo dtrace -p PID -s myapp.d
cargo anyprobe wprp target/release/myapp.exe > myapp.wprp    # wpr -start myapp.wprp -filemode
```

`--provider` and `--probe 'fetch__*'` select probes, and `list --json` gives
one object per definition. The linker can drop the code of a function nothing
calls and keep its description; on Linux, macOS and FreeBSD `list` marks such
a probe and the scripts leave it out. Windows binaries have no per-site
metadata to check against.

## Caveats

- With no tracer attached a probe costs under half a nanosecond. While a
  tracer records it, each firing costs about a microsecond on Linux, macOS
  and FreeBSD, where it traps into the kernel. Keep probes that fire very
  often out of hot loops, or expect the program to slow while they are
  traced.
  [PERFORMANCE.md](https://github.com/jrobhoward/anyprobe/blob/main/docs/PERFORMANCE.md)
  has the measurements.
- A `debug` or `serde` argument runs its `Debug` or `Serialize` impl only
  while a tracer is attached. A panic in that impl is not caught, so
  attaching can surface a bug that never runs otherwise.
- `#[probe]` runs the function's body in a closure, or an awaited `async`
  block for an `async fn`, to capture the return value. It does not support
  `const fn`, functions that return `!`, `#[track_caller]`, or functions
  that return a future without being `async fn` (`#[async_trait]`).
- Methods are named after the function, so two methods named `new` share probe
  names unless one sets `name = "..."`. If their arguments differ, each site
  still passes its own. On macOS a dtrace script tells them apart by the
  function it reports (`probefunc`); on FreeBSD both report `new`, and only
  the probe id differs. bpftrace turns on only one of them. `cargo anyprobe
  list` warns about such probes, and its scripts print no arguments for
  them.
- `anyprobe::list()` reads the executable or library it is linked into, not
  shared libraries loaded alongside it. Each probe's description takes
  about 150 bytes, most of it the source file path and module path.

- macOS: `sudo dtrace` attaches with System Integrity Protection on, for
  binaries that are not signed with the hardened runtime.
- FreeBSD: the program registers its probes with the kernel as it starts.
  DTrace has to be loaded by then (`kldload dtraceall`), and the program has
  to be able to open `/dev/dtrace/helper`, which by default only root and
  `wheel` can. Otherwise the program runs with its probes off, and
  `anyprobe::registration()` says why. Other architectures than x86-64 get
  no probes.
- Linux: perf needs Linux 4.20 or later and a perf that passes the SDT
  semaphore to the kernel; an older perf lists the probe and records nothing.
- Linux: SystemTap needs the binary linked with GNU ld, or with lld and
  `-C link-arg=-Wl,-z,separate-loadable-segments`. bpftrace and perf need
  neither.
- Linux: a Rust `dylib` crate (not `cdylib`) that defines probes needs
  `RUSTFLAGS="--cfg anyprobe_dylib"`, which makes each enabled check slightly
  slower.

The full list of limitations is in
[docs/GAPS.md](https://github.com/jrobhoward/anyprobe/blob/main/docs/GAPS.md).

## Documentation

- [Usage walkthroughs](https://github.com/jrobhoward/anyprobe/tree/main/docs/usage):
  attaching bpftrace, dtrace (macOS and FreeBSD) and ETW to the `demo`
  example.
- [PERFORMANCE.md](https://github.com/jrobhoward/anyprobe/blob/main/docs/PERFORMANCE.md):
  the cost of a probe with and without a tracer, per platform.
- [GAPS.md](https://github.com/jrobhoward/anyprobe/blob/main/docs/GAPS.md):
  every known limitation, including dynamic libraries, panics and size
  limits.
- [ALTERNATIVES.md](https://github.com/jrobhoward/anyprobe/blob/main/docs/ALTERNATIVES.md):
  how anyprobe compares with `usdt`, `probe`, `tracelogging`, `tracing` and
  attaching by symbol.
- [ARCHITECTURE.md](https://github.com/jrobhoward/anyprobe/blob/main/docs/ARCHITECTURE.md):
  how the crates and backends fit together.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
