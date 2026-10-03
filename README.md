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

Status: pre-release. This is the low-level `probes!` macro. An attribute that
probes a function's entry and return, with `serde` and `Debug` arguments, is
planned; see [docs/PLAN.md](docs/PLAN.md).

## Example

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
# handle(1, "/");
```

Each probe becomes a module with `enabled()` and `fire(...)`. Keep anything
that costs time to compute inside the `enabled()` branch.

Arguments are `u8` to `u64`, `usize`, `i8` to `i64`, `isize`, `bool`, raw
pointers, `&str` and `&[u8]`, up to six values per probe (`&str` and `&[u8]`
count as two: pointer and length). The provider defaults to the crate name;
DTrace does not allow one that ends in a digit.

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

## Caveats

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
