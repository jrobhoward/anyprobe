# Changelog

Notable changes to `anyprobe`. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

The first release. `anyprobe`, `anyprobe-macros` and `cargo-anyprobe` are
released together with the same version.

### Added

- Backends, chosen by target:
  - Linux (x86-64, AArch64): SystemTap SDT notes with semaphores, for
    bpftrace and perf. The semaphores' section starts on a page of its own
    (4 KiB on x86-64, 64 KiB on AArch64), so the kernel raises the copy the
    program reads; this costs up to a page of padding. The notes are
    retained, so `--gc-sections` keeps every function that holds a probe
    site instead of leaving a note that names an address outside the code.
    A library whose probed functions are all unused is larger for it.
    A Rust `dylib` crate that defines probes needs `--cfg anyprobe_dylib`.
  - macOS (x86-64, AArch64): DTrace USDT probes, built into DOF by the
    linker.
  - FreeBSD (x86-64): DTrace USDT probes. A constructor builds DOF from a
    table of probe sites and registers it with the kernel through
    `/dev/dtrace/helper` when the executable or library loads, and removes
    it at unload. Registration needs DTrace loaded and access to the device,
    by default root and `wheel` only; without them the program runs with its
    probes off.
  - Windows: ETW TraceLogging events. Each provider name is one ETW
    registration, shared by every probe that uses it in the process, and
    made on the first enabled check of one of its probes.
  - Every other target, FreeBSD on architectures other than x86-64
    included: probes compile to nothing.
- `probes!`: defines probes, each a module with `enabled()` and `fire(..)`,
  to fire from anywhere. Arguments are integers up to 64 bits, `bool`,
  `char`, raw pointers, `&str`, `&[u8]`, `Option<&str>`, `Option<&[u8]>`
  and `&CStr`, at most five values per probe (a `&str`, `&[u8]` or `Option`
  of one counts as two). Five, not six, because DTrace on Apple Silicon
  reads a sixth value as 0.
- `anyprobe::fire!(probe(args))`: fires a `probes!` probe only when it is
  enabled, so its arguments are evaluated only while a tracer is attached.
- `#[probe]`: probes a function's entry and return (`name__entry`,
  `name__return`). The native `probes!` types, references to scalars,
  `&mut str` and `&mut [u8]` are passed as they are; other arguments are
  listed in `debug(..)` (`{:?}`), `serde(..)` (JSON) or `skip(..)`, and an
  unlisted one is a compile error that names these. Encoding runs only while
  a tracer is attached. Encoded values are NUL-terminated strings, cut at
  4096 bytes, and a cut value ends with `...`. Arguments that would take
  more than five values are passed as one JSON object. Other options: `name`,
  `provider`, `native(..)` and `ret = native | serde | debug`.
  - On an `async fn`, the entry probe fires when the body starts and the
    return probe when it completes, and both pass an invocation id first so
    a tracer can pair them when calls interleave.
  - `unwind` adds `{name}__unwind`, which fires when a panic unwinds through
    the function, or when an `async fn`'s future is dropped before
    completing.
  - `symbol` and `symbol = "..."` export the function under a stable,
    unmangled name (`{provider}__{name}` by default), never inlined, for
    tools that attach by symbol. Generic functions and `async fn` are
    rejected; methods of non-generic trait impls are accepted.
  - `const fn`, functions that return `!`, `#[track_caller]` and functions
    that return a future without being `async fn` (`impl Future`, or the
    `Pin<Box<dyn Future>>` that `#[async_trait]` writes) are rejected.
- Provider names: the provider defaults to the crate name, with a `_` after
  it when the name ends in a digit, which DTrace does not allow (`http2`
  becomes `http2_`). `provider = "..."` sets another; one that ends in a
  digit is rejected. Provider and probe names that D reserves (`int`,
  `string`, `uint64_t`, ...) are rejected on every target, since macOS fails
  to link them.
- `Native`: lets `native(..)` pass a type alias or newtype as one 64-bit
  value.
- `anyprobe::next_id()`: an id unique in the process and never 0, to pass to
  related probes so a tracer can pair them. The invocation ids of `#[probe]`
  on an `async fn` come from the same counter.
- `anyprobe::registration()` and `RegistrationError`: whether the probes of
  this executable or library are registered with the tracer, and if not,
  why. Only FreeBSD registers at startup; every other target returns `Ok`.
- `anyprobe::BACKEND`: the name of the backend compiled for the target.
- `anyprobe::list()` and the `registry` module: every probe in the binary,
  with its provider, name, arguments and their types, and the file and line
  that define it. The macros write each description into a section of the
  binary at compile time; nothing runs at startup. `registry::parse` reads
  the same section from a file, and accepts only names the macros write, so
  a description read from an untrusted file can be printed or written into
  a script as it is.
- Features: `serde` (default) for the `serde(..)` encoding, and `autoref`,
  which encodes arguments `#[probe]` does not list as JSON or `{:?}` instead
  of rejecting them. `autoref` implies `serde`, and code that compiles
  without it encodes the same with it.
- `cargo-anyprobe`: `cargo anyprobe list` lists the probes in a built
  binary for any target without running it, warns when one probe name has
  different arguments in different functions, and marks probes whose code
  the linker removed (Linux, macOS and FreeBSD). `bpftrace`, `dtrace` and
  `wprp` write a bpftrace script, a D script and a WPR profile that print or
  record every probe. `--bin` and `--example` build the binary first;
  `--provider` and `--probe` select probes; `list --json` gives one object
  per definition. `bpftrace` needs the binary's path to hold only ASCII
  letters, digits and `/._-+`.
- Examples `demo`, which fires a few probes per second until stopped, and
  `overhead`, which times probed calls with or without a tracer.
- Documentation: a walkthrough per platform that attaches the native tracer
  to `demo` (`docs/usage/`), the cost of a probe with and without a tracer
  (`docs/PERFORMANCE.md`), a comparison with `usdt`, `probe`,
  `tracelogging` and `tracing` (`docs/ALTERNATIVES.md`), and every known
  limitation (`docs/GAPS.md`).
