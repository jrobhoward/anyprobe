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
| Windows | ETW TraceLogging | WPR, PerfView, logman |
| anything else, FreeBSD included | none; probes compile to nothing | none |

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
`myapp:handle__return(ret)`. Integers, `bool`, `char`, raw pointers, `&str`
and `&[u8]` are passed as they are. Other arguments are listed in
`debug(..)` (encoded with `{:?}`), `serde(..)` (encoded as JSON) or
`skip(..)`; an unlisted one is a compile error that names these fixes. The
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
    if request__start::enabled() {
        request__start::fire(id, path);
    }
    // ...
}
```

Each probe becomes a module with `enabled()` and `fire(...)`. Keep anything
that costs time to compute inside the `enabled()` branch. `probes!` takes
the native types only, up to six values per probe (`&str` and `&[u8]` count
as two: pointer and length).

The provider defaults to the crate name. DTrace does not allow one that ends
in a digit, so a crate named like `http2` sets `provider = "..."`.

## Attaching

Linux, with bpftrace (`str(ptr, len)` reads a `&str`):

```text
sudo bpftrace -p PID -e 'usdt:/path/to/binary:myapp:request__start { printf("%d %s\n", arg0, str(arg1, arg2)); }'
```

macOS, with dtrace (DTrace shows `__` in a probe name as `-`):

```text
sudo dtrace -p PID -n 'myapp$target:::request-start { printf("%d %s\n", arg0, copyinstr(arg1, arg2)); }'
```

Windows: the provider is named after the probe provider (`myapp`), with the
standard TraceLogging GUID derived from that name. PerfView accepts `*myapp`;
`logman` and `tracelog` need the GUID. The provider registers with ETW the
first time any of its probes is checked.

Select probes by provider and probe name. DTrace also reports the containing
function, but that is the mangled Rust symbol and changes between builds.

An encoded argument (`debug`, `serde`, or arguments combined into one JSON
object) is a string: a pointer and a length, followed by a NUL byte. bpftrace
reads it with `str(ptr, len)`, dtrace with `copyinstr(ptr, len)`; perf and
gdb read it as a NUL-terminated string. Encoded values are cut at 4096
bytes, and bpftrace reads 64 by default (`BPFTRACE_MAX_STRLEN`). A native
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
calls and keep its description; on Linux and macOS `list` marks such a probe
and the scripts leave it out. Windows binaries have no per-site metadata to
check against.

## Caveats

- `#[probe]` runs the function's body in a closure, or an awaited `async`
  block for an `async fn`, to capture the return value. It does not support
  `const fn`, functions that return `!`, `#[track_caller]`, or functions
  that return a future without being `async fn` (`#[async_trait]`).
- Methods are named after the function, so two methods named `new` share
  probe names unless one sets `name = "..."`. If their arguments differ,
  each site still passes its own; with dtrace a script tells them apart by
  the function it reports (`probefunc`). Nobody has tried this with bpftrace
  yet. `cargo anyprobe list` warns about such probes, and its scripts print
  no arguments for them.
- `anyprobe::list()` reads the executable or library it is linked into, not
  shared libraries loaded alongside it. Each probe's description takes
  about 150 bytes, most of it the source file path and module path.

- macOS: `sudo dtrace` attaches with System Integrity Protection on, for
  binaries that are not signed with the hardened runtime.
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
