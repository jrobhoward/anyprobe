# Changelog

Notable changes to `anyprobe`. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## 0.9.0 - unreleased

### Added

- FreeBSD x86-64 backend: DTrace USDT probes that `dtrace` attaches to with
  `-p` or `-c`. Each site is a `nop` or `xor eax, eax` plus a record in the
  `anyprobe_sites` section; a constructor turns the records into DOF and
  registers it with the kernel through `/dev/dtrace/helper` when the
  executable or library loads, and unregisters it at unload. Registration
  needs DTrace loaded and access to the device, by default root and `wheel`
  only; without them the program runs with its probes off. FreeBSD on other
  architectures still compiles probes to nothing. `cargo anyprobe list`
  marks probes with no site in a FreeBSD binary, and there is a FreeBSD
  walkthrough (`docs/usage/freebsd.md`) and cost section in
  `docs/PERFORMANCE.md`.
- `anyprobe::registration()` and `RegistrationError`: whether the probes of
  this executable or library are registered with the tracer, and if not,
  why. Only FreeBSD registers at runtime; every other target returns `Ok`.
  On Windows a provider registers on the first enabled check of one of its
  probes, so a refusal there is not reported.
- Documentation: walkthroughs that attach bpftrace, dtrace and ETW to the
  new `demo` example (`docs/usage/`), the cost of a probe with and without
  a tracer on each platform (`docs/PERFORMANCE.md`), and a comparison with
  `usdt`, `probe`, `tracelogging` and `tracing` (`docs/ALTERNATIVES.md`).
  `docs/GAPS.md` adds dynamic libraries, dependencies, panics, size limits
  and what attaching changes in the program.
- Examples `demo`, which runs until stopped and fires a few probes per
  second, and `overhead`, which times probed calls with or without a tracer.
- `anyprobe::list()` and the `registry` module: every probe in the binary,
  with its provider, name, arguments and their types, and the file and line
  that define it. The macros write each description into a section of the
  binary at compile time; nothing runs at startup. `registry::parse` reads
  the same section from a file.
- `cargo-anyprobe`: `cargo anyprobe list` lists the probes in a built binary
  for any target without running it, warns when one probe name has
  different arguments in different functions, and marks probes whose code
  the linker removed (Linux, macOS and FreeBSD). `bpftrace`, `dtrace` and
  `wprp` write a bpftrace script, a D script and a WPR profile for them.
- `#[probe]` on `async fn`: the entry probe fires when the body starts, the
  return probe when it completes, and both pass an invocation id first so a
  tracer can pair them across interleaved calls.
- `#[probe(unwind)]`: a `{name}__unwind` probe that fires when a panic
  unwinds through the function, or when an `async fn`'s future is dropped
  before completing.
- `#[probe(symbol)]` and `symbol = "..."`: export the function under a
  stable, unmangled name (`{provider}__{name}` by default), never inlined,
  for tools that attach by symbol.
- `#[probe]`: probes a function's entry and return (`name__entry`,
  `name__return`). Options: `name`, `provider`, `serde(..)`, `debug(..)`,
  `skip(..)`, `native(..)` and `ret = native | serde | debug`. Integers,
  `bool`, `char`, raw pointers, `&str`, `&[u8]` and references to scalars
  are passed natively; any other argument must be listed. Encoded values are
  NUL-terminated strings, cut at 4096 bytes; a cut value ends with `...`.
  Arguments that would take more than six values are passed as one JSON
  object.
- `Native`: lets `native(..)` pass a type alias or newtype as one 64-bit
  value.
- Features: `serde` (default) for JSON encoding; `autoref` to encode unlisted
  arguments as JSON or `{:?}` instead of rejecting them.
- `probes!` accepts `char`.
- Windows: probes share one ETW registration per provider name across the
  process, instead of one per `probes!` block.
- `probes!`: defines probes, each a module with `enabled()` and `fire(...)`.
  Arguments are integers up to 64 bits, `bool`, raw pointers, `&str` and
  `&[u8]`, at most six values per probe. The provider defaults to the crate
  name, with a `_` after it when it ends in a digit, which DTrace does not
  allow (`http2` becomes `http2_`); `provider = "..."` sets another, and one
  that ends in a digit is rejected. The same default applies to `#[probe]`.
- Linux (x86-64, AArch64): SystemTap SDT notes with semaphores, for bpftrace
  and perf. `--cfg anyprobe_dylib` for Rust `dylib` crates.
- macOS (x86-64, AArch64): DTrace USDT probes built by the linker.
- Windows: ETW TraceLogging events; the provider registers with ETW on the
  first enabled check of any of its probes.
- Every other target, FreeBSD on architectures other than x86-64 included:
  probes compile to nothing.

### Fixed

- `probes!`: a module with many probes no longer takes compile memory that
  grows with the square of their number. Each probe's module imported its
  parent's names, which `cargo check` of 3,000 probes in one module took
  3.1 GB for; it now imports them only for a raw pointer to a type named
  relative to the caller's module, and took 0.55 GB.
- `cargo anyprobe dtrace`: the D script set `strsize` to 4096, one byte short
  of the longest encoded value and its NUL, so dtrace dropped the last byte
  of a value cut at the limit. It now sets 4097.
- Linux: `cargo anyprobe list` found no probes in a binary built with rustc
  1.88 and linked with GNU ld, unless the binary called `anyprobe::list()`.
  That rustc does not mark the registry records as retained, so
  `--gc-sections` dropped them. Each probe site now refers to the registry
  section, which keeps it in any binary with a probe site.

- Linux: probes could stay off under perf and `bpftrace -c`, depending on
  how rust-lld laid out the binary. The kernel raises a semaphore by its file
  offset, and when the page holding the semaphores was also the last page of
  the RELRO segment, it raised a copy the program never reads. The
  semaphores' section now starts on a page of its own (4 KiB on x86-64,
  64 KiB on AArch64), at the cost of up to that much padding. `bpftrace -p`
  was not affected.
