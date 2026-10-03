# Architecture

How the workspace is laid out and why each backend works the way it does.
Limitations are in [GAPS.md](GAPS.md); the pre-1.0 design history is in
[PLAN.md](PLAN.md) until it is split into these two files.

## Crates

| Crate | Role |
|---|---|
| `anyprobe-macros` | The `probes!` macro and the `#[probe]` attribute. Target-independent: it checks the input, computes every string a backend needs and emits one `define_probe!` per probe. |
| `anyprobe` | The facade and runtime. One backend module per platform, the encoder, the registry and the public `Native` trait. All `asm!` and `unsafe` live here. |
| `cargo-anyprobe` | The `cargo anyprobe` binary. Reads the registry and the tracer metadata from a built file and writes scripts. |
| `anyprobe-check` | Unpublished. One probe per argument kind, so a library build reaches every backend's code generation. |
| `anyprobe-spike` | Unpublished. Hand-written probes, the attach scripts and the FreeBSD prototype. |

The macros and the runtime are released together with an exact version
requirement, so generated code always matches the runtime it calls.
`__private` is outside the semver contract for the same reason.

## Macros

`probes!` and `#[probe]` produce the same thing: a module per probe holding
`enabled()` and `fire(..)`, defined by the backend's `macro_rules!`. The macro
crate never emits `asm!` or `unsafe`, so a change to a platform's probe format
is a change in one backend file.

`#[probe]` puts its probe modules and their `#[cold]` helpers inside the
function body and runs the original body through `call_once`. Encoding lives in
the cold helpers so that the code on the hot path is the enabled check and
nothing else. Generated code is compiled under the caller's lints, so it uses
absolute paths, `__anyprobe_`-prefixed names and the `allow`s it needs.

## Backends

| Target | Probe | Enabled check | Registration |
|---|---|---|---|
| Linux | SystemTap SDT v3 note from `asm!` | the SDT semaphore, in `.probes` | none |
| macOS | DTrace USDT, as linker relocations the linker turns into DOF | DTrace's is-enabled probe | none |
| Windows | ETW TraceLogging event, one per probe | an atomic flag the provider's enable callback sets | on the first enabled check, `atexit` unregisters |
| other | nothing | `false` | none |

### Linux

Notes come from `asm!` rather than a C helper, so the build needs no C
compiler. A site uses numeric local labels, so inlining or monomorphization
can copy it and each copy is another site under the same probe name.

The semaphore is the enabled check, and the only way a probe knows a tracer
is attached. The kernel raises it by file offset, in the first writable
mapping of that file page. rust-lld writes the RELRO and data segments back to
back, so each object emits one retained, page-aligned byte in `.probes` to keep
the semaphores off the RELRO page. Sites that pass pointers are `readonly`,
never `nomem`, since the tracer reads memory through them.

### macOS

Call sites are `asm!` calls to `__dtrace_probe$...` and
`__dtrace_isenabled$...`. ld64 rewrites them to `nop` and builds the DOF
section, so the runtime registers nothing. A probe site must be an `asm!`
call and not a Rust call: a Rust call that ends a function compiles to a tail
call, and the rewritten site would fall through into the next function.

### Windows

The provider is `tracelogging_dynamic`, so each probe is an event whose
metadata is built at runtime from the same strings the other backends use.
One provider registration is shared by every probe of that name in the
process, since ETW limits registrations per process. The enable callback
updates every probe's flag under a lock; a probe added later copies the
provider's current state under the same lock, so no change is lost.

### Other targets

Probes compile to nothing, and the enabled check is the constant `false`, so
arguments are never computed. FreeBSD is in this group.

## Argument encoding

`serde` and `debug` values are written into a thread-local buffer and passed as
a pointer and a length, with a NUL after the bytes. Values are cut at 4096
bytes on a character boundary. A probe that fires while another is being
written on the same thread is dropped, not panicked on.

`autoref` picks `Serialize`, then `Debug`, by method resolution on a wrapper
in the caller's crate, since the macro cannot see types. It is additive: code
that compiles without the feature encodes the same with it, because Cargo
unifies features across the dependency graph.

## Registry

Each probe definition also emits one record: a `#[used]` static byte array in
a section of its own, built at compile time from a `concat!` the macro writes.
A record holds no pointers, so `anyprobe::list()` in the running program and
`cargo anyprobe` on a file read the same bytes, for any target, with no
relocation processing. This is why the registry is not a `linkme` slice, whose
elements hold pointers.

| Format | Section |
|---|---|
| ELF | `anyprobe_probes`, a C identifier, so `__start_` and `__stop_` bound it |
| Mach-O | `__DATA,__anyprobe` with `no_dead_strip`, bounded by `section$start` and `section$end` |
| PE | `.aprobe$b` between `.aprobe$a` and `.aprobe$c` markers |

The parser skips zero padding between records, rejects an unknown version, and
never panics on malformed input. A change to the record layout bumps
`registry::VERSION`, and the macros, the parser and `cargo-anyprobe` change
with it.

## `cargo anyprobe`

It reads a built file with `goblin` (ELF, Mach-O including fat files, PE) and
`dof` (DOF sections), and never runs the file. Where the file carries tracer
metadata (SDT notes, DOF), `list` compares it with the registry to mark a probe
whose code the linker removed. `--bin` and `--example` run `cargo build` and
take the executable from its JSON messages.

## Verification

Unit and integration tests run everywhere. Compile-fail cases run under
trybuild; those whose output is rustc's own wording run on a pinned toolchain.
Behaviour a tracer sees is checked by the attach scripts in
`spike/scripts/`, once per OS, with exact event counts.
