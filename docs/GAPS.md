# Known gaps

Each section is one limitation: what it is, why it exists, and what changing
it costs. `README.md` lists the ones that change how the crate is used.

## Platform coverage

### Intel Macs

x86-64 macOS builds are checked without root on an Apple Silicon host: every
probe site rewritten by ld64 and inside its function, and `cargo anyprobe
list` finding every probe with a site. No tracer has attached to one on an
Intel Mac, and none is planned: the backend stays, CI keeps building it, and
it is unsupported in the sense that nobody has checked it. `spike/scripts/attach-macos-attr.sh`
covers it. Cost: one run on an Intel host.

### ARM64 hosts

AArch64 Linux and Windows on ARM64 compile (the cross-target loops build
them), and CI runs the attach checks on GitHub's `ubuntu-24.04-arm` (bpftrace)
and `windows-11-arm` (ETW) runners. Nobody has run a tracer against either by
hand. The Linux SDT code uses the 64 KiB section alignment AArch64 needs;
that value is untested on a kernel with 64 KiB pages.

### FreeBSD

FreeBSD has a backend on x86-64 only. FreeBSD on AArch64 and other
architectures compiles to the no-op backend, and no support is planned:
nobody has checked whether `fasttrap` supports USDT there. Cost: an AArch64
FreeBSD machine, and AArch64 versions of the two site instructions.

The FreeBSD measurements come from FreeBSD 15.0 in a KVM virtual machine.
Nobody has run the backend on FreeBSD hardware or on another release. The
CI job runs in a virtual machine too.

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
process. bpftrace and perf need neither and are the tested tracers. CI does not run
SystemTap, which needs building from source on current kernels;
`spike/scripts/attach-linux-stap.sh` checks it by hand.

### perf and older kernels

perf attaches only on Linux 4.20 or later with a perf that passes the SDT
semaphore to the kernel. An older one lists the probe and records nothing.
The semaphore is the only way a probe knows a tracer is attached, so there is
no fallback.

## Arguments

### Encoded values are cut

An encoded argument (`debug`, `serde`, collapsed arguments) is cut at 4096
bytes, at a character boundary, and then ends with `...` in place of its
last bytes. A value that ends with `...` on its own is indistinguishable
from a cut one. The tracer may read less, and nothing marks that cut:
bpftrace reads 64 bytes unless `BPFTRACE_MAX_STRLEN` is raised, and dtrace
256 unless `strsize` is raised (the scripts `cargo anyprobe dtrace` writes
set it to 4097, the longest value and its NUL). The limit is a constant;
making it configurable would cost a load in the cold path only.

### Large native strings and byte slices

A native `&str` or `&[u8]` is passed as a pointer and a length, with no copy
and no limit. On Linux, macOS and FreeBSD the tracer copies what it reads,
up to its own string limit, so a large value costs the program nothing
extra. On
Windows the value is copied into the event: TraceLogging cuts a string or
byte field at 65535 bytes, and ETW drops any event larger than 64 KB, or
larger than the session's buffer size, without telling the program. The
event is dropped first, so the cut is never seen in a trace. Checked on
Windows 11 with `logman` (`check-gaps-windows.ps1`): a `&str` of 65,350
bytes was recorded whole and one of 65,400 bytes was not recorded at all,
as were 65,535 and 70,000. Nothing in between was recorded cut. The limit
for a string field is a little under 65,400 bytes, the rest of the 64 KB
going to the event's headers and other fields. A per-backend cap on native
values would keep such events, at the cost of silently shortening them.

### Five values per probe

SDT and DTrace pass probe values in registers, six of them on macOS x86-64,
but DTrace on Apple Silicon reports the sixth (`arg5`) of a USDT probe as 0
although the site passes it in `x5`; the `pid` provider's `arg5` is not
affected, and `uregs[R_X5]` reads the value. The `usdt` crate reports the
same thing as issue #62. So every target stops at five: `probes!` takes the
native types only and rejects a probe that needs more, and `#[probe]`
collapses arguments that would pass more into one JSON object. If macOS
fixes `arg5`, the limit can go back to six without breaking any probe;
`spike/scripts/check-gaps-macos.sh` checks whether it still reads 0. Going
past six needs a different argument passing scheme per backend.

### Native `&str` has no terminator

A native `&str` is a pointer and a length with no NUL after it. bpftrace and
dtrace read it with the length. perf and gdb read a NUL-terminated string and
read past the end. A `&CStr` argument is one pointer to NUL-terminated bytes,
which those tools read as written.

### `None` and an empty value look alike

An `Option<&str>` or `Option<&[u8]>` that is `None` is passed as a null
pointer and length 0. On Linux, macOS and FreeBSD a tracer can tell it from
`Some("")` by the pointer; on Windows ETW has no absent field, so `None` is
recorded as an empty string or no bytes, the same as `Some("")`. The scripts
`cargo anyprobe` writes print `None` as `(none)` under dtrace, which errors on
`copyinstr` of a null pointer, and as an empty string under bpftrace, which
reads nothing for length 0.

### Provider names ending in a digit

DTrace does not allow one. The macros reject a `provider = "..."` that ends
in a digit, and add a `_` to a default taken from a crate name that does, so
a crate named `http2` has the provider `http2_`. Its probe names then differ
from the crate name by that `_`; setting `provider` picks another name. A
crate name of 58 bytes that ends in a digit is one byte too long once the
`_` is added and has to set `provider`.

### Names DTrace reserves

On macOS ld64 writes each provider and its probes into a D declaration and
compiles it, so a provider or probe name that D reserves fails to link with
"Could not compile reconstructed dtrace script" and "error creating dtrace
DOF section". The macros reject D's reserved words (`int`, `string`,
`probe`, `this`, ...) and the integer types D defines (`int8_t` to
`uint64_t`, `intptr_t`, `uintptr_t`) on every target, so a name that builds
on one platform builds on all of them. The names of types in the kernel's
type data fail the same way and are not rejected, since that set changes
between macOS releases: on macOS 27 these include `size_t`, `pid_t`,
`off_t` and `kern_return_t`. A probe named after one builds on every other
platform and fails to link on macOS. `#[probe]` names end in `__entry`,
`__return` or `__unwind` and never collide.

## `#[probe]`

### Unsupported functions

`#[probe]` runs the body in a closure (or an awaited `async` block), so it
rejects `const fn`, `-> !`, `#[track_caller]` and functions that return a
future without being `async fn` (`#[async_trait]`). `symbol` also rejects
generic fns, trait-impl methods and `async fn`, which have no single stable
symbol. Supporting any of them changes how the return value is captured.

### `async fn` probes

An `async fn`'s entry probe fires when its body first runs and its return
probe when the body completes. Nothing fires at each poll or while the future
is pending, so time spent waiting cannot be told apart from time spent
running. Probes per poll would need a wrapper future around the body.

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
- On macOS `dtrace -l` lists one row per function that contains a site,
  named by its mangled symbol, which changes between builds. Scripts select
  probes by provider and name. On FreeBSD all sites of one probe definition
  are one row.
- A function that nothing calls after optimization has no site, but its
  registry record stays; `cargo anyprobe list` marks it on Linux, macOS and
  FreeBSD.

`symbol` keeps one function out of line, with one stable symbol.

### Methods with the same name

Methods are named after the function, so two `new` methods share probe names
unless one sets `name = "..."`. Arguments that differ are still passed
correctly at each site, but a script has to tell the sites apart by function.
`cargo anyprobe list` warns about the case and the generated scripts print no
arguments for it. On macOS DTrace's `probefunc` is the mangled symbol, which
names the impl. On FreeBSD it is the function's name, `new` for both, so only
the probe id tells them apart; each keeps its own argument types.

bpftrace 0.20 attaches to every site but turns on only one of them. Each
definition has its own SDT semaphore, and bpftrace raises the semaphore of
one site only, so the other functions' enabled checks stay false and their
probes never fire, under `-p` and `-c` alike. Reading an argument that one
of the functions does not pass fails: `arg1` when one passes a single value
is a compile error ("couldn't get argument 1"), and `arg0` when one passes
none crashed bpftrace. `attach-linux-attr.sh` checks the first part.

Setting `name = "..."` on all but one avoids all of this. Sharing one
semaphore between every definition of a provider and probe name would let
bpftrace turn them all on, but not read their arguments. A
`#[probe_impl(prefix = "...")]` that names every method in an `impl` block
would save writing `name` on each.

### `autoref`

The `autoref` feature picks `Serialize`, then `Debug`, by type checking in the
caller's crate. It is additive: whatever compiles without it encodes the same
with it. A type that implements neither is a compile error from rustc, worded
by rustc and pinned only on one toolchain.

## Building

### Many probes that point at local types

Each probe from `probes!` is a module. A probe with a raw pointer to a type
named relative to the caller's module (`*const Request`, or a `c_void`
brought in with `use`) glob-imports its parent so the type resolves as it
does next to the macro. rustc's work for that import grows with the number
of names in the parent, so many such probes in one module cost the square of
their number at compile time: `cargo check` peaked at 3.0 GB for 3,000 of
them, against 0.55 GB for 3,000 probes with integer arguments, which import
nothing. Writing the pointee as `crate::Request` or `::core::ffi::c_void`, or
splitting the probes across modules, avoids it. A type alias in the parent
would avoid the import, but an alias cannot hold a lifetime left out of the
type (`*const Wrapper` for a `Wrapper<'a>`), which a pointer argument can.

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
library loads, a site table in a FreeBSD `.so` that the library's own
constructor registers, and an ETW provider in the DLL that unregisters when
it unloads. Tracers attach to the library's file (`usdt:/path/lib.so:...`)
or to a process that loaded it.

A `cdylib` loaded with `dlopen` has been traced on Linux (bpftrace -p),
FreeBSD and macOS (dtrace). On FreeBSD and macOS it was traced alongside the
executable, both using one provider name: dtrace lists the probe once per
module and fires it in each, and `probemod` tells them apart. On macOS dyld
registers the library's DOF when it loads. On FreeBSD the library's probes
went away at `dlclose` when it had a provider of its own (see [Shared
providers outlive `dlclose`](#shared-providers-outlive-dlclose)); nobody has
checked `dlclose` on macOS. On Linux, bpftrace 0.20.2
cannot attach to a probe name that both the executable and a library in the
process define: through the executable it reports "Could not resolve symbol",
through the library "couldn't get argument 1". A probe name that only the
library defines works. A library that gives its probes a provider name of its
own avoids both problems. On Windows a DLL loaded with `LoadLibraryW` was
traced with one `logman` session (`check-gaps-windows.ps1`): events of a
provider only the DLL defines were recorded, and events of a provider both
the executable and the DLL define were recorded from each, since each module
registers its own copy of the provider under the one GUID. `anyprobe::list()` called from the library lists the
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

On Linux, macOS and FreeBSD each firing of a traced probe is a trap into the
kernel; on Windows it is a system call. A probe that fires often slows the
program while traced, and a predicate in the tracer runs after the trap. When
the tracer's buffers fill, events are dropped rather than the program blocked.
[PERFORMANCE.md](PERFORMANCE.md) has the measurements.

### What a tracer writes into the process

On Linux, macOS and FreeBSD the kernel writes a breakpoint over each site of a
traced probe, in the process's own copy of the code page, and on Linux raises
the probe's semaphore. It removes both when the tracer exits, including when
the tracer is killed: after `kill -9` of bpftrace or `perf record` on Linux,
and of dtrace on FreeBSD and macOS, every site held its original
instruction again, the semaphore was 0, and the program reported the probe
off (`spike/scripts/check-gaps-linux.sh`, `check-gaps-freebsd.sh`,
`check-gaps-macos.sh`). On macOS this holds for `dtrace -p` too: the program
keeps running. On FreeBSD, killing a `dtrace -p` also kills the program; see
[Killing `dtrace -p` kills the program](#killing-dtrace--p-kills-the-program). A site placed wrongly would
put the breakpoint in the middle of an instruction; the attach checks confirm
each site is a `nop` (Linux) or a rewritten call (macOS) inside its function,
for release and fat LTO builds. On FreeBSD the site table holds the address of
each site's own label, which the assembler places on the instruction. On
Windows nothing in the code changes: ETW calls the provider's enable callback,
which sets a flag per probe.

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

## FreeBSD runtime

### Registration can fail

The probes reach the kernel through a constructor that runs as the
executable or library loads. It fails if DTrace is not loaded yet (a
program started before `kldload dtraceall` has no probes until it restarts)
or if the program may not open `/dev/dtrace/helper`, which is `root:wheel`,
mode `0660`, by default. The program then runs with its probes off. C
programs built with `dtrace -G` are in the same position, since `drti.o`
opens the same device, and they give up silently. `anyprobe::registration()`
returns the reason instead; nothing is printed. A devfs rule that opens the
device to the program's group changes this, and only an administrator can
add one.

### Shared providers outlive `dlclose`

A library unloaded with `dlclose` unregisters its probes, and the kernel
removes them, unless another object in the process uses the same provider
name. `fasttrap` removes a provider only when the last object using it
unregisters, so until then `dtrace -l` still lists the library's probes. C
programs behave the same way. A library that may be unloaded can use a
provider name of its own.

### Killing `dtrace -p` kills the program

`dtrace -p PID` holds the process with ptrace for the whole session. With
`kern.kill_on_debugger_exit=1`, FreeBSD's default, the kernel kills a traced
process whose tracer exits without detaching, so `kill -9` on that dtrace
kills the program too (`check-gaps-freebsd.sh`). `truss -p` does the same;
nothing in the crate can change it. Ctrl-C detaches normally. A probe
description that names the process id (`demo1234:::tick`) instead of using
`-p` and `$target` does not take hold of the process, and killing that
dtrace leaves the program running with its probes off. On macOS, killing a
`dtrace -p` leaves the program running with its probes off
(`check-gaps-macos.sh`).

### Startup work

Each executable or library that links anyprobe parses its site table, builds
DOF and makes one `ioctl` before `main` (or before `dlopen` returns), with an
allocation for the DOF that lives as long as the object. In a virtual machine
that took about 0.03 ms for 10 probes and 9 ms for 3,000, plus 0.03 to 4.5 ms
for the `ioctl` ([PERFORMANCE.md](PERFORMANCE.md#freebsd-startup)). With
DTrace loaded, a parent that waits for the program sees about 1.4 ms more
per run, after the program has exited. A long-running program pays this
once; a short program run many times in a loop pays it each time. Building
the DOF before the program runs would need a step between compiling and
linking, as `dtrace -G` is for C, which Cargo does not have.

### Argument types in `dtrace -l -v`

`dtrace -l -v` shows "Argument Types: None" for every probe, though the DOF
carries each argument's C type and typed `args[N]` works in a D program.
FreeBSD 15.0's `dtrace` shows the same for a `pid` probe, so this looks like
how it lists probes rather than something the crate could change.

## Windows runtime

### Provider registration

A provider registers with ETW the first time any of its probes is checked and
unregisters through `atexit`. In a DLL the C runtime runs `atexit` handlers at
unload, which is what keeps ETW from calling into unmapped code. At exit, if
another thread holds the provider lock, the handler gives up rather than
wait, since that thread may never release it.

If ETW refuses a registration (the per-process limit, or no memory), the probe
that asked stays off for the life of the process, and the next probe naming
that provider that has not been checked yet asks again. Nothing reports the
refusal: `anyprobe::registration()` returns `Ok` on Windows, since nothing has
registered when a program calls it at startup. Registering every provider at
startup would let it report one, at the cost of registrations for providers
whose probes never fire, each counted against the per-process limit.

### Per-process limits

ETW limits the registrations per process, so one provider is shared by every
probe that names it, however many crates define probes.
