# Known gaps

Each section is one limitation: what it is, why it exists, and what changing
it costs. `README.md` lists the ones that change how the crate is used.

## Platform coverage

### Intel Macs

x86-64 macOS builds are checked without root on an Apple Silicon host: every
probe site rewritten by ld64 and inside its function, and `cargo anyprobe
list` finding every probe with a site. No tracer has attached to one on an
Intel Mac. `spike/scripts/attach-macos-attr.sh` covers it. Cost: one run on
an Intel host.

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
makes the binary larger. With the default layout SystemTap can place a
uprobe at the wrong address, where the breakpoint can crash the traced
process. bpftrace and perf need neither and are the tested tracers.

### perf and older kernels

perf attaches only on Linux 4.20 or later with a perf that passes the SDT
semaphore to the kernel. An older one lists the probe and records nothing.
The semaphore is the only way a probe knows a tracer is attached, so there is
no fallback.

## Arguments

### Encoded values are cut

An encoded argument (`debug`, `serde`, collapsed arguments) is cut at 4096
bytes, at a character boundary, without a marker. The tracer may read less:
bpftrace reads 64 bytes unless `BPFTRACE_MAX_STRLEN` is raised, and dtrace
256 unless `strsize` is raised (the scripts `cargo anyprobe dtrace` writes
set it to 4096). A flag operand to say "truncated" would use one of the six
argument slots. The limit is a constant; making it configurable would cost a
load in the cold path only.

### Large native strings and byte slices

A native `&str` or `&[u8]` is passed as a pointer and a length, with no copy
and no limit. On Linux and macOS the tracer copies what it reads, up to its
own string limit, so a large value costs the program nothing extra. On
Windows the value is copied into the event: TraceLogging cuts a string or
byte field at 65535 bytes, and ETW drops any event larger than 64 KB, or
larger than the session's buffer size, without telling the program. A
per-backend cap on native values would keep such events, at the cost of
silently shortening them.

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

### Closures and functions without a body

`#[probe]` applies to a function item with a body: a free function, a method,
or a trait method with a default body. A closure, or a trait method
declaration without a body, cannot carry it; `probes!` probes fired from
inside the closure or each implementation cover those.

### Probe sites multiply

Each inlined copy of a probed function, and each instantiation of a generic
one, is another site under the same probe name. Every tracer handles this,
and it is why a probed function can still be inlined. Costs:

- bpftrace attaches one uprobe per site, one at a time, so attaching takes
  longer, and over a window of a few milliseconds the sites go live at
  slightly different moments. Counts compared across sites over a short
  window can disagree by a few firings.
- `dtrace -l` lists one row per function that contains a site, named by its
  mangled symbol, which changes between builds. Scripts select probes by
  provider and name.
- A function that nothing calls after optimization has no site, but its
  registry record stays; `cargo anyprobe list` marks it on Linux and macOS.

`symbol` keeps one function out of line, with one stable symbol.

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

## Linking and dependencies

### Rust `dylib` crates

A `dylib` crate that defines probes needs `--cfg anyprobe_dylib` on Linux,
which replaces the direct semaphore load with a plain Rust load. That makes
each enabled check slightly slower (0.98 ns instead of 0.77 ns for two probes
on the machine that measured it). A `cdylib`, a `staticlib` and an executable
need nothing. The cfg only changes the Linux semaphore load; the macOS and
Windows backends have none.

### Shared libraries

A shared library built from Rust (`cdylib`) carries its own probes: SDT notes
in the `.so`, DOF in the `.dylib` that the dynamic loader registers when the
library loads, and an ETW provider in the DLL that unregisters when it
unloads. Tracers attach to the library's file (`usdt:/path/lib.so:...`) or
to a process that loaded it. Nobody has run a tracer against a probe in a
shared library yet. `anyprobe::list()` called from the library lists the
library's probes, and `cargo anyprobe` reads the library file like an
executable.

### Transitive dependencies

A probe defined in a library is compiled into every binary that links the
library, however deep in the dependency graph, and the binary needs no
dependency on anyprobe of its own. The provider defaults to the name of the
crate that defines the probe, so a library's probes keep their names in
every program. Cargo unifies anyprobe's features across the graph; `autoref`
is additive so that one crate enabling it does not change another's probes.

### Two anyprobe versions in one binary

Cargo can link two semver-incompatible anyprobe versions into one binary.
Each version's probes work on their own. Their registry records share one
section, and a record whose format version a reader does not know stops
`anyprobe::list()` and `cargo anyprobe` with an error at that record, so
the probes after it are not listed. On Windows each version registers its
own copy of a provider name, and both count against the per-process limit.
Only one record format exists so far.

## Panics

### A panicking encoder

A `debug` or `serde` argument is encoded by its `Debug` or `Serialize`
impl, which runs only while a tracer is attached. A panic there is not
caught: it unwinds out of the probed function as if the function had
panicked, or aborts the process under `panic = "abort"`. Attaching a tracer
can therefore turn a latent bug in a `Debug` impl into a failure. Catching it
would need `catch_unwind` in each cold helper, which hides the bug and
does nothing under `panic = "abort"`.

### A panic skips the return probe

Without `unwind`, a panic in the body leaves an entry probe firing with no
return. Scripts that pair entries with returns have to allow for it.
`unwind` adds a probe that fires instead.

### `unwind` and `panic = "abort"`

The unwind probe of a sync function fires while a panic unwinds through it.
Under `panic = "abort"` the process aborts before that, so the probe never
fires. The unwind probe of an `async fn` still fires when its future is
dropped before completing, since that is not a panic.

## Attaching

### Attaching runs code that otherwise does not run

With no tracer attached, a probe's arguments are never evaluated. Attaching
runs the cold helpers, and with them every `Debug` and `Serialize` impl the
probes encode. An impl that panics, blocks on a lock the caller holds, or
takes a long time affects the program only while it is traced. The `Debug`
impls of `Mutex` and `RefCell` print a placeholder rather than block or
panic when the value is locked or borrowed.

### Each firing costs about a microsecond

On Linux and macOS each firing of a traced probe is a trap into the kernel;
on Windows it is a system call. A probe that fires often slows the program
while traced, and a predicate in the tracer runs after the trap. When the
tracer's buffers fill, events are dropped rather than the program blocked.
[PERFORMANCE.md](PERFORMANCE.md) has the measurements.

### What a tracer writes into the process

On Linux and macOS the kernel writes a breakpoint over each site of a traced
probe, in the process's own copy of the code page, and on Linux raises the
probe's semaphore. It removes both when the tracer exits, including when the
tracer is killed. A site placed wrongly would put the breakpoint in the
middle of an instruction; the attach checks confirm each site is a `nop`
(Linux) or a rewritten call (macOS) inside its function, for release and
fat LTO builds. On Windows nothing in the code changes: ETW calls the
provider's enable callback, which sets a flag per probe.

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
