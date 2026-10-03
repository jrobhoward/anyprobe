# Known gaps

Each section is one limitation: what it is, why it exists, and what changing
it costs. `README.md` lists the ones that change how the crate is used.

## Platform coverage

### macOS verification of the registry

The registry sections, `anyprobe::list()` through `section$start`, the DOF
cross-check in `cargo anyprobe list` and the generated D scripts have been
built for macOS targets and checked at the object level, but not run on a Mac
since the registry was added. `spike/scripts/attach-macos-attr.sh` covers
them. Cost: one run on each of an Apple Silicon and an Intel host.

### ARM64 hosts

AArch64 Linux and Windows on ARM64 compile (the cross-target loops build
them) but nobody has run a tracer against either. The Linux SDT code uses the
64 KiB section alignment AArch64 needs; that value is untested on hardware.

### FreeBSD

FreeBSD compiles to the no-op backend. The DOF prototype in `spike/` registers
probes through `/dev/dtrace/helper` and has never run on FreeBSD. Cost: a
FreeBSD machine, and answers to the open questions in `docs/PLAN.md`.

### Hardened and protected processes

macOS binaries signed with the hardened runtime cannot be traced, and
`dtrace` needs System Integrity Protection relaxed for others. Neither can be
changed from inside the crate.

## Tracers

### SystemTap

SystemTap needs file offsets equal to addresses. rust-lld does not produce
that layout by default; the binary has to be linked with GNU ld or with
`-C link-arg=-Wl,-z,separate-loadable-segments`. The separate-segments layout
makes the binary larger. bpftrace and perf need neither and are the tested
tracers.

### perf and older kernels

perf attaches only on Linux 4.20 or later with a perf that passes the SDT
semaphore to the kernel. An older one lists the probe and records nothing.
The semaphore is the only way a probe knows a tracer is attached, so there is
no fallback.

### Rust `dylib` crates

A `dylib` crate that defines probes needs `--cfg anyprobe_dylib`, which
replaces the direct semaphore load with a plain Rust load. That makes each
enabled check slightly slower (0.98 ns instead of 0.77 ns for two probes on
the machine that measured it). A `cdylib` and an executable need nothing.

## Arguments

### Encoded values are cut

An encoded argument (`debug`, `serde`, collapsed arguments) is cut at 4096
bytes without a marker. bpftrace reads 64 bytes unless
`BPFTRACE_MAX_STRLEN` is raised. A flag operand to say "truncated" would use
one of the six argument slots.

### Six values per probe

SDT and DTrace pass six values in registers. `probes!` takes the native types
only and rejects a probe that needs more; `#[probe]` collapses arguments that
would pass six into one JSON object. Raising the limit needs a different
argument passing scheme per backend.

### Native `&str` has no terminator

A native `&str` is a pointer and a length with no NUL after it. bpftrace and
dtrace read it with the length. perf and gdb read a NUL-terminated string and
read past the end.

### Provider names ending in a digit

DTrace does not allow one, so the macros reject it at compile time. A crate
named like `http2` has a default provider that ends in a digit and sets
`provider = "..."`. The macros do not rename it, since the provider name is
part of the stable probe name.

## `#[probe]`

### Unsupported functions

`#[probe]` runs the body in a closure (or an awaited `async` block), so it
rejects `const fn`, `-> !`, `#[track_caller]` and functions that return a
future without being `async fn` (`#[async_trait]`). `symbol` also rejects
generic fns, trait-impl methods and `async fn`, which have no single stable
symbol. Supporting any of them changes how the return value is captured.

### Methods with the same name

Methods are named after the function, so two `new` methods share probe names
unless one sets `name = "..."`. Arguments that differ are still passed
correctly at each site, but a script has to tell the sites apart by function.
`cargo anyprobe list` warns about the case and the generated scripts print no
arguments for it. Only DTrace (`probefunc`) has been checked; bpftrace has
not.

### `autoref`

The `autoref` feature picks `Serialize`, then `Debug`, by type checking in the
caller's crate. It is additive: whatever compiles without it encodes the same
with it. A type that implements neither is a compile error from rustc, worded
by rustc and pinned only on one toolchain.

## Registry and `cargo anyprobe`

### Executable only

`anyprobe::list()` reads the registry of the executable or library it is
linked into. Shared libraries loaded later are not listed, and `cargo
anyprobe` reads one file at a time and has no `--pid`. Adding either means
walking the loaded images (`dl_iterate_phdr`, dyld, `EnumProcessModules`) and
a section lookup in each.

### Windows has no site check

SDT notes and DOF record which probes have a site in the binary, so `list`
marks a probe whose code the linker removed. A Windows binary builds its
TraceLogging metadata at runtime, so there is nothing to compare against, and
a removed probe is listed as present.

### Record size

Each record is about 150 bytes, most of it the source file and module paths.
A binary with many probes carries that data in a read-only section. Shorter
paths would need the macro to rewrite `file!()`.

### Record format changes

A record holds no pointers and has a version. A reader rejects an unknown
version instead of guessing, so a `cargo-anyprobe` older than the crate that
built the binary stops with an error. `cargo-anyprobe` depends on `anyprobe`
with an exact version for the same reason.

## Windows runtime

### Provider registration

A provider registers with ETW the first time any of its probes is checked and
unregisters through `atexit`. In a DLL the C runtime runs `atexit` handlers at
unload, which is what keeps ETW from calling into unmapped code. At exit, if
another thread holds the provider lock, the handler gives up rather than
wait, since that thread may never release it.

### Per-process limits

ETW limits the registrations per process, so one provider is shared by every
probe that names it, however many crates define probes.
