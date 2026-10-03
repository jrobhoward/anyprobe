# anyprobe — plan

Working plan for the pre-1.0 crate. Exempt from the writing-style rules in
`CLAUDE.md`; once the design settles it is split into `docs/ARCHITECTURE.md`
and `docs/GAPS.md` and deleted.

Cross-platform, USDT-style function probes for Rust. Annotate a function with
`#[anyprobe::probe]` and it gets stable-named entry/return probes that a tracer
can attach to in a running process. With no tracer attached the cost is one
enabled-check per probe; inlining is unaffected.

## Goals

- One attribute, three OSes: Linux, macOS, Windows (no-op elsewhere).
- FreeBSD is deferred indefinitely (see [FreeBSD](#freebsd-deferred)): it
  compiles to the no-op backend until someone needs it.
- Stable probe names, independent of Rust paths and symbol mangling.
- Near-zero disabled cost: argument encoding only runs when a tracer is attached.
- Explicit, predictable argument encoding; optional automatic selection behind a
  feature flag.

## Crates

Cargo workspace, edition 2024, `rust-version = 1.88.0`, MIT OR Apache-2.0.

| Crate | Path | Role |
|---|---|---|
| `anyprobe` | `crates/anyprobe` | Facade and runtime: re-exports the macro; per-OS backends as `cfg`-selected modules. Features below. |
| `anyprobe-macros` | `crates/anyprobe-macros` | `#[probe]` attribute (proc macro). Compile-fail behaviour pinned by trybuild tests in `tests/ui/`. |

### Cargo features (`anyprobe`)

| Feature | Default | Effect |
|---|---|---|
| `serde` | on | Enables `serde(...)` encoding (pulls `serde`, `serde_json`). |
| `autoref` | off | Unlisted args/return pick an encoding automatically (see below). Forwards to `anyprobe-macros/autoref`. |
| `async` | on | `async fn` support (phase 3). |

## Attribute syntax

```rust
anyprobe::provider!("myapp");   // optional; default provider = CARGO_CRATE_NAME

#[anyprobe::probe]
fn parse_request(id: u64, path: &str) -> u32 { ... }
// probes: myapp:parse_request__entry(id, path_ptr, path_len)
//         myapp:parse_request__return()

#[anyprobe::probe(
    name = "db_query",   // probe base name (default: fn name)
    serde(q),            // JSON-encode these args
    debug(opts),         // `{:?}`-encode these args
    skip(conn),          // omit these args
    native(id),          // force native encoding (e.g. type aliases)
    ret = debug,         // encode return value: native | serde | debug (omitted = no payload)
    unwind,              // opt-in: fire `__unwind` on panic / async cancellation
    symbol,              // opt-in: #[inline(never)] + stable exported symbol
)]
fn query(conn: &mut Conn, id: RowId, q: &Query, opts: &Opts) -> Rows { ... }
```

Methods: the macro cannot see `Self`, so probes in `impl` blocks use the fn
name unless `name = "..."` is given; `#[anyprobe::probe_impl(prefix = "Conn")]`
on the `impl` block may supply a prefix (later).

## Argument encoding

### Modes

| Mode | Applies to | Passed to the probe as | Tracer view |
|---|---|---|---|
| `native` | ints, `bool`, `char`, raw pointers, `&str`, `&[u8]` | registers/operands; slices as `(ptr, len)` | direct (`arg0`, `str(arg1, arg2)`) |
| `serde` | `T: Serialize` | JSON in a thread-local buffer, `(ptr, len)` | string; `json()` on illumos DTrace |
| `debug` | `T: Debug` | `{:?}` in a thread-local buffer, `(ptr, len)` | string |
| `skip` | anything | nothing | — |

ETW maps `native` to typed TraceLogging fields and `serde`/`debug` to string
fields.

### Default selection, `autoref` feature **off** (default)

1. Args named in `serde(..)`, `debug(..)`, `skip(..)`, `native(..)` use that mode.
2. Unlisted args whose type is *syntactically* a known primitive (`u8..u128`,
   `i8..i128`, `usize`, `isize`, `bool`, `char`, `&str`, `&[u8]`, `*const T`,
   `*mut T`, and `&`/`&mut` of these) use `native`.
3. Any other unlisted arg is a compile error naming the arg and the fixes:
   `add serde(x), debug(x), or skip(x) — or enable the anyprobe "autoref" feature`.

`native` args get a generated `anyprobe::Native` trait-bound assertion, so a
wrong `native(x)` is a compile error, not a miscompiled probe.

### Default selection, `autoref` feature **on**

Rule 3 changes: unlisted non-primitive args dispatch via autoref specialization,
picking the first that applies: `Native` > `Serialize` > `Debug` > compile error.

- Enabling the feature only turns errors into working code; anything that
  compiles with it off encodes identically with it on. That keeps Cargo's
  feature unification safe.
- In generic code, dispatch uses the *declared bounds*, not the concrete type
  (`T: Debug` encodes as `debug` even if the concrete `T` is `Serialize`).
  Document this.
- Libraries that rely on `autoref` must enable it themselves.

### Payload limits

- SDT allows ≤12 operands and DTrace ~10. Native args use 1 operand, or 2 for slices
  and encoded args. If a probe would exceed the limit, all non-skipped args collapse
  into one JSON/Debug object `{ "q": ..., "opts": ... }`, passed as `(ptr, len)`.
- Buffers are thread-local and reused (no per-event allocation); max size
  configurable, with truncation flagged by a `truncated` bit in a flags operand.
- bpftrace reads strings with a length cap (default 64; `BPFTRACE_MAX_STRLEN`).
- perf and gdb read a string argument only as NUL-terminated (`+0(%si):string`
  in `perf probe`, `(char *)$_probe_arg1` in gdb); neither can take a
  `(ptr, len)` pair. bpftrace (`str(ptr, len)`) and SystemTap
  (`user_string_n`) can. Proposal, not decided: write a NUL after every
  `serde`/`debug` payload in the thread-local buffer (one byte, already
  behind the enabled check), so perf and gdb read encoded args as strings;
  `len` still excludes it. Native `&str` cannot be terminated without a copy,
  so docs say which tools read it.

## Expansion (sync fn)

```rust
fn parse_request(id: u64, path: &str) -> u32 {
    if anyprobe::__enabled!(myapp, parse_request__entry) {      // load+cmp+branch / is-enabled nop
        __anyprobe_parse_request_entry_cold(id, path);           // #[cold] #[inline(never)]
    }
    let __ret = (move || -> u32 { /* original body */ })();     // `return`/`?` keep their meaning
    if anyprobe::__enabled!(myapp, parse_request__return) {
        __anyprobe_parse_request_return_cold(&__ret);
    }
    __ret
}
```

The cold helpers own all encoding work so the hot path stays small. Probe sites
are emitted with numeric local asm labels, so inlining, monomorphization, and
LLVM duplication each produce another site under the same probe name.

## Backends

| OS | Probe emission | Enabled check | Registration | Attach with |
|---|---|---|---|---|
| Linux | SystemTap SDT v3 notes (`.note.stapsdt`) via `asm!` | SDT semaphore (`.probes`, kernel ≥4.20 ref_ctr) | none | bpftrace, perf (SystemTap: link flag needed, see Phase 0 results) |
| macOS | DTrace USDT via linker relocations (`__dtrace_probe$…`, `__dtrace_isenabled$…`) | DTrace is-enabled | none (ld64 builds DOF) | `dtrace` (SIP: `--without dtrace`) |
| FreeBSD (deferred) | no-op; the spike's DOF prototype is kept in `spike/` | `false` | — | — |
| Windows | ETW TraceLogging via `tracelogging_dynamic`; one event per probe | one atomic flag, kept in sync by the provider's enable callback | lazily, on the first enabled check of any probe in the provider; unregistered at exit (`atexit`) | WPR/WPA, PerfView `*myapp`, `tracelog`, DTrace `etw` |
| other | nothing | `false` | — | — |

Names: one canonical name, rendered per backend (DTrace `__` → `-`; ETW provider
GUID = TraceLogging name-hash of the provider name, event name = probe name).

Explicit `anyprobe::init()` exists as a fallback when static constructors are
undesirable.

## Async fns (phase 3)

```rust
#[anyprobe::probe(debug(req), ret = debug, unwind)]
async fn handle(req: Request) -> Response { ... }
```

- Body is wrapped in `anyprobe::ProbedFuture`, which fires
  - `__entry` on first poll (args were moved into the future, still available),
  - `__return` on `Poll::Ready`,
  - optionally `__poll` / `__pending` (`poll` flag) for scheduling-latency analysis,
  - `__unwind` on drop-before-completion if `unwind` is set (cancellation or panic,
    distinguished by `std::thread::panicking()` in a flags operand).
- **Invocation id:** async probes carry a `u64` id as their first operand so
  tracers can correlate interleaved tasks. The id comes from a global atomic,
  fetched only if some probe of the fn is enabled at first poll (otherwise 0).
- Out of scope initially: `async-trait`, fns returning `impl Future` (later via
  a `future` flag).

## Opt-in extensions

### `unwind`

A drop guard with a `completed` flag. If the guard drops without completing, it fires
`__unwind` (panic for sync fns; panic or cancellation for async). Cost when the
probe is off: a flag store plus a drop check. No effect under `panic = "abort"`
for sync fns.

### `symbol`

For raw uprobes / DTrace `pid` provider / Frida use without USDT:

- adds `#[inline(never)]` and `#[unsafe(export_name = "<provider>__<name>")]`
  (or `symbol = "custom"`); the `unsafe(...)` form is required in edition 2024.
- compile errors for generic fns, trait-impl methods, and async fns (no single
  stable symbol exists).
- Only the outermost fn gets the symbol; the USDT probes still fire normally.

## Tooling (phase 4)

Superseded by "Phase 4" below.

- Probe registry via `linkme` distributed slice (name, provider, arg names and
  encodings, file:line), `anyprobe::list()`.
- `cargo anyprobe list` and generators for bpftrace scripts, D scripts, and WPR
  profiles.
- Optional `tracing` interop layer.

## Phases

0. **Spike (go/no-go)**: hand-written probes in a tiny binary on all four OSes,
   attached by the native tools, plus criterion benchmarks of disabled overhead
   (target < ~1 ns/call versus an unannotated fn). Questions to settle:
   - SDT semaphores with bpftrace `-p PID`
   - notes surviving LTO, `codegen-units=1`, and duplicated sites
   - macOS relocation emission from `asm!`
   - FreeBSD helper registration from Rust
   - `tracelogging` cost and registration
1. `anyprobe` runtime and the low-level `probes!` macro for Linux, macOS and
   Windows, no-op elsewhere (FreeBSD included). Design below.
2. `#[probe]` for sync fns: naming, `native`/`serde`/`debug`/`skip`, `ret`,
   compile-error diagnostics; `autoref` feature.
3. `async fn`, `unwind`, `symbol`. Design and status below.
4. Tooling.
5. CI matrix:
   - Linux: privileged bpftrace
   - Windows: ETW session as admin
   - macOS: metadata presence only, since SIP is on in CI
6. Documentation from real hosts: replace the Linux and Windows walkthrough
   output and fill in the attached-cost numbers with captures from those
   machines. Design and status below.

## Phase 1: runtime and `probes!`

### Scope

- `crates/anyprobe-macros`: the function-like `probes!` proc macro.
- `crates/anyprobe`: re-exports `probes!`; `#[doc(hidden)] pub mod __private`
  holds the per-backend `macro_rules!` that generated code calls, and the
  Windows provider runtime.
- `crates/anyprobe-check` (unpublished): a library that defines one probe per
  argument kind, so `cargo build --lib --target ...` reaches codegen for every
  backend's `asm!` without a linker.
- An example (`crates/anyprobe/examples/work.rs`) with the spike's probes,
  labels and command line, so the spike's attach scripts and benchmark check
  the real crate (`ATTACH_CRATE=anyprobe`).
- Not in phase 1: `#[probe]`, `serde`/`debug` encoding, `autoref`, async.

### API

```rust
anyprobe::probes! {
    provider = "myapp";   // optional; default CARGO_CRATE_NAME

    /// Docs land on the generated module.
    pub fn request__start(id: u64, path: &str);
    fn request__done(id: u64, status: u16, ok: bool);
}

if request__start::enabled() {
    request__start::fire(id, path);
}
```

Each declaration becomes a module with the declaration's visibility,
holding `enabled() -> bool`, `fire(...)` with the declared parameters, and
`PROVIDER` / `NAME` string constants. Both functions are `#[inline(always)]`:
every inlined copy of `fire` is another probe site, and every copy of
`enabled` another check. Callers keep encoding behind `enabled()`; phase 2's
`#[probe]` generates exactly this shape with cold helpers.

Each block expands to one hidden wrapper module holding the block's provider
and its probe modules, and re-exports each probe module with its declared
visibility. The probes reach the provider with `super::`, which only works
because both are inside the wrapper: a module nested in a function body
resolves `super::` past the function's scope, so a provider defined next to
the block would be unreachable there (found when the first doctest, which
runs inside `fn main`, was checked for Windows). Phase 2 needs one provider
per crate rather than per block (ETW limits registrations per process), so it
adds a crate-root `anyprobe::provider!()` that `#[probe]` refers to by
absolute path.

### Argument types

Recognized syntactically, since the macro must know each argument's shape to
build the probe metadata. Type aliases are rejected; phase 2's `native(x)`
covers them.

| Rust type | Linux / macOS operands | DTrace C type | ETW field |
|---|---|---|---|
| `u8`, `u16`, `u32`, `u64`, `usize` | 1, widened to `u64` (`8@`) | `uint64_t` | `u8`..`u64` (`usize` as `u64`) |
| `i8`, `i16`, `i32`, `i64`, `isize` | 1, widened to `i64` (`-8@`) | `int64_t` | `i8`..`i64` (`isize` as `i64`) |
| `bool` | 1, as `u64` | `uint64_t` | `bool32` |
| `*const T`, `*mut T` | 1, address as `u64` | `uintptr_t` | `u64`, hex |
| `&str` | 2: pointer, length | `char *`, `uint64_t` | `str8`, UTF-8 |
| `&[u8]` | 2: pointer, length | `uint8_t *`, `uint64_t` | `binary` |

At most 6 operands per probe: macOS x86-64 passes probe arguments in the six
System V argument registers. (SDT allows 12.) Phase 2 collapses larger argument
lists into one encoded object.

### Names

- Provider: ASCII letters, digits and `_`, not starting with a digit, at most
  58 bytes (DTrace appends the pid), and not ending in a digit (`dtrace -h`:
  "provider name may not end with a digit"). A crate whose name breaks this
  needs `provider = "..."`.
- Probe: ASCII letters, digits and `_`, not starting with a digit, at most 63
  bytes. Used as-is in SDT notes and ETW events; DTrace shows `__` as `-`.

### Code generation

The proc macro is target-independent: it validates, computes every string
each backend needs (SDT argument format, DTrace symbol names with hex-encoded
C types, ETW field calls, register assignments for both macOS architectures),
and emits one `::anyprobe::__private::define_probe! { ... }` per probe. The
`macro_rules!` behind that name is defined once per backend in `anyprobe`
under `cfg`, and uses only the parts it needs. All `asm!` and `unsafe` live in
`anyprobe`, where they are reviewed and tested once, not in proc-macro output.

### Windows registration

No static constructor. Each provider is a `static` holding a lazily created
`tracelogging_dynamic::Provider` and one `AtomicU8`: 0 off, 1 on, 2 not yet
registered (initial). `enabled()` is one relaxed load: 0 and 1 answer
directly; 2 takes a cold path that registers the provider once (with an
`atexit` handler that unregisters it, which `register`'s safety contract needs
for providers in DLLs) and stores the current state. ETW reports existing
sessions to a provider as it registers, so a session started before the first
check still enables it. The enable callback stores the state on every change.
Cost: the provider is invisible to ETW until the first check runs.

### Verification

- Unit tests for name validation and string computation; trybuild tests for
  every rejected input.
- `anyprobe-check` codegen for every target; clippy on every target.
- The example under the existing attach scripts: macOS (`inspect_dof` and
  `sudo dtrace`), Linux (bpftrace, perf), Windows (ETW), and the disabled-cost
  benchmark compared against the spike.

### Status (2026-10-02)

Implemented. Checked on the macOS host:

- `work` example (spike's probes through `probes!`): `inspect_dof` passes for
  arm64 and x86_64, release and release-lto, with the same sites as the
  spike (3 `work__entry` probe sites, 1 `work__return`).
- Linux: the example linked statically for x86_64 (musl, rust-lld) has the
  same four SDT notes as the spike, each on a `nop`, with shared semaphores
  and an executable `.stapsdt.base`. `anyprobe-check` notes on x86_64 and
  aarch64 carry the expected formats (`-8@` for signed values, an empty
  format for no arguments, six operands at the limit).
- Codegen for every target; clippy on every target, including
  `--cfg anyprobe_dylib`; MSRV 1.88; `cargo deny`; publish dry run of both
  crates.
- macOS attach: `ATTACH_CRATE=anyprobe spike/scripts/attach-macos.sh` with
  `sudo dtrace -c`, SIP on, Apple Silicon: release and release-lto each gave
  exactly 40 firings per label over 40 iterations and 160 `work-return`.
- Disabled cost on Apple Silicon: 1.266 ns probed against 0.950 ns baseline,
  the same as the spike's hand-written probes (1.273 ns).
- 39 macro unit tests, 13 compile-fail cases, 7 runtime integration tests,
  doctests including README.md.

Found while implementing: a provider defined next to the probe modules is
unreachable when `probes!` is used in a function body (above); Windows was
the only backend that referenced it, so only a Windows build showed it. A
test now declares probes in a function body, and clippy for Windows compiles
it.

Needs the other machines (`ATTACH_CRATE=anyprobe`):

- Linux x86_64: done, see the Linux table below.
- Linux aarch64: bpftrace and perf attach (`attach-linux.sh`,
  `attach-linux-perf.sh`).
- Windows: done, see the table below.

## Phase 2: `#[probe]` for sync fns

### Expansion

```rust
#[anyprobe::probe(serde(q), ret = debug)]
fn query(id: u64, q: &Query) -> Result<Rows, E> { body }
// becomes
fn query(id: u64, q: &Query) -> Result<Rows, E> {
    mod __anyprobe {                     // inside the body: no outside names needed
        /* define_probe! modules __anyprobe_entry (query__entry) and
           __anyprobe_return (query__return) */
        #[cold] #[inline(never)] pub fn fire_entry(id: u64, q: Value<'_>) { /* encode, fire */ }
        #[cold] #[inline(never)] pub fn fire_return(ret: Value<'_>) { /* encode, fire */ }
    }
    if __anyprobe::__anyprobe_entry::enabled() {
        __anyprobe::fire_entry(id, ::anyprobe::__private::serde_value!("q", &q));
    }
    let __anyprobe_ret = ::anyprobe::__private::call_once(move || -> Result<Rows, E> { body });
    if __anyprobe::__anyprobe_return::enabled() {
        __anyprobe::fire_return(Value::debug(&__anyprobe_ret));
    }
    __anyprobe_ret
}
```

- The body runs through `call_once(impl FnOnce() -> R)`, not `(move || ..)()`:
  a closure called directly is inferred `FnMut`, and then a `&mut self`
  method cannot return a borrow of `self`. With `FnOnce` every tested shape
  compiles: early `return`, `?` into `Box<dyn Error>`, elided lifetimes,
  `impl Trait` returns (annotation omitted, inferred), generics, `self` by
  value / `&` / `&mut`, `mut` arguments, trait default methods, `unsafe fn`
  (body wrapped in `unsafe {}` with `unused_unsafe` allowed, since pre-2024
  editions allow unsafe calls directly in an `unsafe fn`).
- The helpers are not generic. Encoded arguments cross into them as
  `Value<'_>`: an enum of the native kinds plus `&dyn Debug` and
  `&dyn SerializeJson` (object-safe `Serialize`). Generic fns therefore get
  one helper and one probe site, not one per instantiation, and the probe
  module never names the user's types (pointers are passed as `*const ()`).
- Rejected: `async fn` (phase 3), `const fn`, `-> !`, `#[track_caller]` (the
  closure would report its own location), pattern arguments other than `_`.
  `unwind` / `symbol` say "not implemented yet".

### Decisions

- Provider: `provider = "..."` per attribute, default `CARGO_CRATE_NAME`. The
  crate-root `anyprobe::provider!()` from the original sketch is dropped: a
  proc macro cannot read it, and a `macro_rules!` callback through `crate::`
  is rejected for macro-expanded `macro_export` macros. See open questions.
- Windows: one ETW registration per provider *name* per process, interned at
  runtime (`etw::Probe` per probe, leaked `Provider` per name). Each probe
  keeps its own `AtomicU8`, which the provider's callback updates, so the
  check stays one load. `probes!` uses the same path; its per-block wrapper
  module and provider static are gone.
- Native types: the phase 1 list plus `char`, references to scalars (passed
  by value), `&mut str`, `&mut [u8]`. `u128`/`i128` are not native (would
  need two operands and a format no tracer reads as one value); use `debug`.
- `native(x)` on an unrecognized type: one unsigned 64-bit operand through the
  public `anyprobe::Native` trait (`to_u64`). Signedness cannot be chosen per
  type: the SDT format is a literal in the `asm!` template.
- `self` is skipped unless listed; `_` arguments are skipped.
- Encoded payloads: thread-local reused `Vec<u8>`; a NUL after each payload
  (the "proposal" above, adopted: perf and gdb read them); cut at 4096 bytes
  at a UTF-8 boundary, no truncation flag (see open questions). A probe fired
  while another is encoding on the same thread gets a fresh buffer.
- Collapse: if the entry probe's operands exceed 6, all non-skipped args go
  into one JSON object passed as `args`: natives as JSON values, `serde` raw,
  `debug` as JSON strings. Works without the `serde` feature.
- `autoref` order is `Serialize` > `Debug` > error. `Native` is not in the
  chain: the operand count must be known to the proc macro, and a
  trait-selected native would change it. `autoref` implies `serde`, so a
  second crate enabling `serde` cannot flip an argument from `Debug` to JSON.
  An omitted `ret` is no payload with or without `autoref` (otherwise turning
  `autoref` on would change a probe that compiled without it).
- The `serde`-off and no-`autoref` errors are `macro_rules!` in
  `anyprobe::__private` (compiled on every target), not decisions in the proc
  macro, so `anyprobe-macros` has no features. The `autoref` fallback carries
  `#[diagnostic::on_unimplemented]` on a method bound, so the error is
  "`Opts` has no probe encoding", not a list of autoref traits.
- Probe names: `{name}__entry` / `{name}__return`; `name` defaults to the fn
  name, so the base can be at most 55 bytes.

### Status (2026-10-02)

Implemented. Checked on the Linux x86_64 host:

- Integration tests (`tests/probe_attr.rs`, `tests/probe_autoref.rs`) cover
  every signature shape above; encoder unit tests cover JSON escaping,
  truncation at a character boundary, NUL termination, buffer reuse and
  reentrancy. 21 new compile-fail cases (`anyprobe-macros/tests/ui`, and
  `anyprobe/tests/ui/<feature-set>` for the four that depend on features).
- Generated code is clean under `clippy::pedantic` and `nursery` in the
  caller's crate.
- Feature sets: default, `--no-default-features`, `autoref`, both, all.
  Found while doing it: the workspace `--no-default-features` command never
  built anyprobe without `serde`, because `anyprobe-macros`'s dev-dependency
  and `anyprobe-check` took the defaults. Both now use
  `default-features = false` (check mirrors the features).
- Clippy and `build --lib` codegen for every target, including Windows with
  the new provider runtime, and `--cfg anyprobe_dylib`; MSRV 1.88; doc;
  `cargo deny` (adds `serde`, `serde_json`; permissive).
- `attr` example SDT notes: one note per probe site, argument formats as
  expected (`8@` pairs for encoded args). Helpers that encode have two sites
  per probe (the fire closure is called from the thread-local path and the
  reentrancy fallback); both are real sites under the same name.

Needs verification:

- bpftrace reading every encoding: `spike/scripts/attach-linux-attr.sh`
  (needs sudo; also in CI on x86_64 and aarch64).
- Windows: the provider runtime changed (interning, per-probe flags). It
  compiles and lints for `x86_64-pc-windows-msvc`; nothing has run it.
  `ATTACH_CRATE=anyprobe attach-windows.ps1` re-checks the `work` example.

### macOS (2026-10-03)

Checked on the macOS host (Apple Silicon, macOS 27), with
`spike/scripts/attach-macos-attr.sh` and by hand:

- `inspect_dof` on the `attr` and `same_name` examples, arm64 and x86_64,
  release and release-lto: every probe and is-enabled site rewritten by ld64
  and inside its function. Unlike Linux, each encoding helper keeps one probe
  site per probe here, not two.
- Same name, different argument types (`same_name`: `Foo::new(x: u32)`,
  `Bar::new()`, `other::new(label: &str)`, all `new__entry`): ld64 links it
  without complaint and writes one DOF entry per containing function, each
  with its own argument types. Identical no-payload return helpers were
  merged by LLVM into one function, so `new__return` has one probe site that
  all three callers reach. DTrace attaches to all three entries and reads
  each one's arguments as that function declares them (below).
- `sudo dtrace -c` with SIP on, release and release-lto, 20 iterations
  (`attach-macos-attr.sh`): every encoding read with exact counts. Native
  `id` and `path`, the native return value, `serde` JSON both by length and
  up to its NUL, `debug` returns (10 `Ok(5)`, 10 `Err("odd N")`), the
  collapsed JSON object, and `debug(self)`. For `same_name`, `Foo::new` read
  `x` as 0..19, `Bar::new` fired 20 times, `other::new` read `label` 20
  times, and `new__return` fired 60 times. The first run reported two
  failures, both in the script: its unexpected-output check did not allow
  dtrace's SIP notice.
- Disabled cost: the `#[probe]` function in the benchmark compiles to the
  same 17-instruction hot path as the hand-written `probed` (two is-enabled
  sites, the frame for the cold calls; only register choices and the helper
  names differ), against 7 for the unprobed baseline. Timings taken on
  battery power with a recent build load were about 3.3 ns for all three and
  the spike alike (0.95 ns baseline on 2026-10-02), so they settle nothing
  below a nanosecond; the instruction comparison does.
- Every Definition of Done check that runs on macOS passes, including clippy
  1.99 on every target, the `ui_rustc` cases on 1.99, every feature set, MSRV
  1.88, `cargo deny` and the publish dry run. The dry run first failed on a
  stale local build of `anyprobe-macros 0.1.0` from an earlier dry run (cargo
  treats a registry crate of one version as immutable); CLAUDE.md now says to
  `cargo clean -p` both crates first.

### Open questions

- Crate-wide provider. A crate whose name ends in a digit (`http2`, `sha2`,
  and every trybuild test crate) cannot use the default and must repeat
  `provider = "..."` on every attribute. Options: append `_` to such names
  by default (silent, but `probes!` would need the same rule); read
  `[package.metadata.anyprobe] provider` from the manifest via
  `CARGO_MANIFEST_DIR` (rebuild tracking unclear); a textually scoped
  `macro_rules!` defined by `anyprobe::provider!` at the crate root (works
  only for modules declared after it, and makes it mandatory).
- Same probe name, different signatures: `Foo::new(x: u32)` and
  `Bar::new()` both default to `new__entry`. SDT and ETW tolerate it. On
  macOS ld64 writes one DOF entry per function and DTrace reads each with its
  own types (see macOS above), so a script must branch on `probefunc` to
  know which arguments it has. Detecting a clash needs crate-wide knowledge
  the macro does not have. Phase 4: `cargo anyprobe list` warns about it,
  from the registry of the linked binary.
- Truncation is silent. A flags operand would cost one of the six, and
  bpftrace's own 64-byte default cuts long before 4096.
- Payload cap is a constant. Configurable at runtime costs a load in the cold
  path only; not done until someone needs it.

## Phase 3: `async fn`, `unwind`, `symbol`

Supersedes the sketches under "Async fns" and "Opt-in extensions" above.

### `async fn`

```rust
#[anyprobe::probe(ret = native)]
async fn fetch(id: u64, path: &str) -> u64 { body }
// becomes
async fn fetch(id: u64, path: &str) -> u64 {
    mod __anyprobe { /* probes and helpers, as for a sync fn */ }
    let __anyprobe_invocation: u64 = if __anyprobe::__anyprobe_entry::enabled() {
        let __anyprobe_invocation = ::anyprobe::__private::next_invocation();
        __anyprobe::fire_entry(__anyprobe_invocation, id, path);
        __anyprobe_invocation
    } else {
        0
    };
    let __anyprobe_ret = async move {
        if false { let __anyprobe_never: u64 = loop {}; return __anyprobe_never; }
        body
    }
    .await;
    if __anyprobe::__anyprobe_return::enabled() {
        __anyprobe::fire_return(__anyprobe_invocation, __anyprobe_ret);
    }
    __anyprobe_ret
}
```

- No future wrapper type. The signature stays an `async fn`, so `Send`,
  lifetimes and `async fn` in traits behave as before; the checks are
  ordinary statements in the body, which runs at the first poll.
- The body is an `async move` block so that its `return`s end the block, not
  the function. An async block takes its output type from its first
  `return`, unlike an `async fn`, which takes the declared type: a body that
  returns `Box::new(1u8)` and then `Box::new("s")` from a function declared
  `-> Box<dyn Debug>` fails to compile in a plain block. The unreachable
  `return` first pins the declared type, so the others coerce, as
  `tracing`'s `#[instrument]` does. Neither a pass-through helper with a
  `Future<Output = R>` bound nor an async closure with `-> R` declared
  fixes this (both tried, on 1.88 and 1.99). `impl Trait` cannot be written
  there; the type is then inferred.
- Invocation id: a global `AtomicU64` from 1, drawn only when the entry
  probe is on, passed as the first value of the entry, return and unwind
  probes. It takes one of the six values, so a probe collapses one argument
  sooner than its sync equivalent. A return with id 0 started while the
  entry probe was off.
- `poll` / `pending` probes from the sketch are not done: they need a
  wrapper future, and no one has asked for scheduling latency yet.

### `unwind`

A guard created after the entry check and `mem::forget`ten after the body
returns, so the normal path runs no drop code. Its `Drop` checks the unwind
probe and fires it. Sync: `{name}__unwind()`, only reachable by a panic.
Async: `{name}__unwind(invocation, panicking)`, also when the future is
dropped before completion; `panicking` tells the two apart.
`std::thread::panicking` is reached through `anyprobe::__private`, so the
caller's crate needs no `::std` path.

### `symbol`

`#[inline(never)]` and `#[unsafe(export_name = "...")]` on the function;
the name is `{provider}__{name}` unless given, and must be ASCII letters,
digits, `_`, `.` and `$`. Rejected by the attribute: `async fn` (the symbol
only creates the future), functions generic over types or consts including
`impl Trait` arguments, and functions already carrying `#[inline]`,
`#[no_mangle]` or `#[export_name]`. rustc itself rejects methods of generic
`impl` blocks ("functions generic over types or consts must be mangled"),
which the attribute cannot see. Methods in trait impls are allowed (rustc
accepts them), unlike the sketch.

### Status (2026-10-03)

Checked on the macOS host:

- `tests/probe_async.rs`: async shapes (native arguments and return, `?`
  into a boxed error, returns of different types coercing to `Box<dyn ..>`,
  borrowed and elided-lifetime returns, `impl Trait`, generics, by-value
  arguments, early `return` of `()`, collapsed arguments, `&mut self` and
  `self` methods, a trait impl), `Send` futures, a future dropped mid-await,
  `unwind` on panicking sync and async functions (the panic still
  propagates), and `symbol` called through both exported names. Expansion
  shape tests in `anyprobe-macros/src/attr_tests.rs`; six new compile-fail
  cases.
- `attr_async` example: every site rewritten and inside its function,
  arm64 and x86_64, release and release-lto; the unwind checks of the
  cancelled future sit in its drop glue; `_attr_async__exported` is a
  global text symbol.
- `sudo dtrace -c` with SIP on, release and release-lto, 20 iterations
  (`spike/scripts/attach-macos-attr.sh`): 40 `fetch` entries and 40
  returns, every return paired with its entry by a unique non-zero
  invocation id and carrying the matching value, with the second call of
  each pair returning first; the cancelled `slow` fired its unwind probe 20
  times with `panicking` 0 and its entry's invocation id, and its return
  probe never; `may_panic` fired entry 20, return 10 and unwind 10 times;
  `exported` was read with ids 0 to 19 both by its USDT probe and by the
  `pid` provider through the symbol `attr_async__exported`. The script's awk
  checks were also tested on simulated broken output.

Checked on Windows 11 (x86_64), release and release-lto, with `logman` and
`tracerpt` (`spike/scripts/attach-windows-attr.ps1`, run elevated; the
`attach-windows.ps1` checks also pass for the spike and the `work` example):
about 150 `attr_async` iterations per session, 300 `fetch` returns each
paired with its entry by invocation id and carrying `id * 10 + path.len()`,
with the calls interleaved; the cancelled `slow` fired its unwind probe with
`panicking` false and its entry's invocation id, and its return probe never;
`may_panic` returned for even ids and unwound for odd ids, matching its
entries; `exported` fired entry and return. ETW decodes `panicking` as
`false`, not 0. A session starts after the process, so the script checks
every recorded event rather than counting them. On Windows `symbol` is only
a symbol name for debuggers; nothing attaches by it.

Checked on Linux x86_64 (kernel 6.8, bpftrace 0.20.2), release and
release-lto (`spike/scripts/attach-linux-attr.sh`, sudo; `attach-linux.sh`
also passes for the spike and the `work` example): the `attr_async` glibc
build has one SDT note per probe, each on a `nop`, and
`attr_async__exported` is a global `FUNC` symbol. Run under `bpftrace -c`
for 20 iterations, so the counts are exact: 40 `fetch` entries and 40
returns, every return paired with its entry by a unique non-zero invocation
id and carrying the matching value, the second call of each pair returning
first; the cancelled `slow` fired its unwind probe 20 times with
`panicking` 0 and its entry's invocation id, and its return probe never;
`may_panic` fired entry 20, return 10 and unwind 10 times; `exported` was
read with ids 0 to 19 both by its USDT probe and by
`uprobe:BIN:attr_async__exported`. `bpftrace -c` raised the semaphores of a
process it started, as `-p` does for a running one. AArch64 Linux is
checked by the same script in CI only.

## Phase 4: registry, `cargo anyprobe`, script generators

Supersedes "Tooling (phase 4)" above. Decided 2026-10-03: the registry is
anyprobe's own link section of pointer-free records, not a `linkme` slice;
`cargo anyprobe` reads built binaries and never runs them; the `tracing`
layer is not in this phase (see "Deferred" below).

### Registry records

Every `define_probe!` module also emits one record, a `#[used]` static
byte array in a dedicated section. A record holds no pointers, so the same
bytes are read at runtime by `anyprobe::list()` and from a file on disk by
`cargo anyprobe`, for any target, with no relocation processing.

```text
record  = magic "APRB" | u16 LE body length | body
body    = fields, each UTF-8 and NUL-terminated:
          version ("1"), provider, name, origin, function, module path,
          file, line, argument count, then per argument: name, type
origin  = probes | entry | return | unwind
type    = u8 u16 u32 u64 usize i8 i16 i32 i64 isize bool char ptr
          str bytes json debug auto object
```

- The proc macro writes the body with `concat!`, so `module_path!()`,
  `file!()` and `line!()` come from the caller; a `const fn` copies it into
  `[u8; 6 + BODY.len()]` behind the header. Built at compile time, nothing
  runs at startup.
- `function` is the annotated function's name for `#[probe]` (empty for
  `probes!`). `type` is how the value reaches the tracer: the Rust integer
  width (every integer is one 64-bit slot on Linux and macOS, its own width
  on ETW), `str` and `bytes` take two slots (pointer, length), and `json`,
  `debug`, `auto` (autoref: JSON or `{:?}`, chosen by type checking, which
  the macro cannot see) and `object` (collapsed arguments) are NUL-terminated
  strings passed like `str`. An `async fn`'s `invocation` and an unwind
  probe's `panicking` are ordinary arguments.
- One record per probe definition, not per site: statics are not
  monomorphized, inlined or duplicated.
- Sections: ELF `anyprobe_probes` (a C identifier, so `__start_` /
  `__stop_` bound it); Mach-O `__DATA,__anyprobe` with `no_dead_strip`
  (`section$start$__DATA$__anyprobe` / `section$end$...`, as `linkme`
  does); PE `.aprobe$b` between `.aprobe$a` and `.aprobe$c`
  markers that the `anyprobe` crate defines (image section names are 8
  bytes at most). The no-op backend emits nothing and `list()` is empty.
- The parser skips zero bytes between records (the MSVC linker may pad
  grouped sections), rejects an unknown version with an error rather than
  guessing, and never panics on malformed input.
- Kept by the linker: on ELF, rustc's `#[used]` sets `SHF_GNU_RETAIN`, and
  a scratch crate showed records surviving `--gc-sections` and fat LTO in
  release and release-lto, including one in a module nothing calls and one
  in a generic function (2026-10-03, rust-lld). On COFF every record's
  object carries an `/INCLUDE:` directive for it (`llvm-readobj
  --coff-directives`), which `/OPT:REF` honours. On Mach-O,
  `no_dead_strip` keeps them through ld64 dead stripping in release and
  release-lto (checked on the host, 2026-10-03).
- Consequence: a probe whose code the linker dropped still has a record.
  `cargo anyprobe` cross-checks records against the tracer metadata (SDT
  notes on Linux, DOF on macOS) and marks probes with no site; scripts skip
  them. Windows builds its TraceLogging metadata at runtime, so there is
  nothing to cross-check there.
- Only the executable's own records: `list()` does not see shared
  libraries loaded later, and `cargo anyprobe` reads one file at a time.

### Runtime API

```rust
pub fn list() -> Probes;                       // every record in this binary
pub fn parse(section: &[u8]) -> Probes<'_>;    // records from other bytes
// Probes: Iterator<Item = Result<ProbeInfo<'a>, RegistryError>>
pub struct ProbeInfo<'a> { provider, name, origin, function, module_path,
                           file, line, args: Args<'a> }   // accessors
pub struct ArgInfo<'a> { name, ty: ArgType }
#[non_exhaustive] pub enum Origin { Probes, Entry, Return, Unwind }
#[non_exhaustive] pub enum ArgType { U8, ..., Str, Bytes, Json, Debug, Auto, Object }
```

`RegistryError` in `error.rs` (`thiserror`, `#[non_exhaustive]`), the
crate's first error type. `ArgType::slots()` (1 or 2) gives the SDT / DTrace
argument index of each argument.

### `cargo anyprobe`

New published crate `cargo-anyprobe` (binary). Dependencies already in the
graph: `goblin` (ELF, Mach-O including fat, PE), `dof` (DOF on macOS),
`serde_json`, `anyprobe` (the parser, `default-features = false`), and
`tracelogging` without macros (`Guid::from_name`, portable). Arguments are
parsed by hand.

```text
cargo anyprobe list     [TARGET] [--json]
cargo anyprobe bpftrace [TARGET] [--provider P] [--probe GLOB]
cargo anyprobe dtrace   [TARGET] [--provider P] [--probe GLOB]
cargo anyprobe wprp     [TARGET] [--provider P]
TARGET = PATH | --bin NAME | --example NAME  (+ --profile, --target, -p)
```

- `--bin` / `--example` run `cargo build --message-format=json` and take
  the executable from the artifact messages; a path is read as is.
- `list`: provider, probe, arguments with types, function and file:line.
  Warns when one probe name has two argument layouts (the open question
  from phase 2): DTrace then gives each function its own types and scripts
  must branch on `probefunc`. Marks records with no site.
- `bpftrace`: one clause per probe, `usdt:BIN:provider:name`, printing the
  probe name and every argument by name: integers with their sign,
  `ptr` in hex, strings with `str(ptr, len)`, `bytes` with `buf(ptr, len)`.
  Run with `sudo bpftrace -p PID FILE`.
- `dtrace`: the same in D, `provider$target:::name-with-dashes`,
  `copyinstr(ptr, len)`. Run with `sudo dtrace -p PID -s FILE`.
- `wprp`: a WPR profile enabling each provider by GUID (the name hash
  TraceLogging uses), for `wpr -start FILE`.
- A probe name with two layouts is printed without arguments in the
  generated scripts, with a comment saying why.

### Verification

- Parser unit tests (round trip, padding, truncation, unknown version,
  invalid UTF-8) on every host.
- `tests/registry.rs`: `list()` in a test binary returns each probe defined
  there, with its arguments, types and source line; empty on no-op targets.
- CLI tests on the `attr`, `attr_async` and `same_name` examples: `list`
  output, the same-name warning, and generated scripts compared with
  expected files.
- Attach: on Linux, run the generated bpftrace script against the `attr`
  example (added to `attach-linux-attr.sh`); on macOS the generated D
  script; on Windows `wpr -start` with the generated profile.
- Hosts: the registry sections on Mach-O and PE, read back by
  `cargo anyprobe list` on a host build (ld64 dead stripping, MSVC
  `/OPT:REF`).

### Status (2026-10-03)

Implemented: records from both macros (`probes.rs` `registry_record`),
`anyprobe::registry` (`list`, `parse`, `ProbeInfo`, `ArgType`, `Origin`)
and `RegistryError`, and the `cargo-anyprobe` crate with `list` (text and
`--json`), `bpftrace`, `dtrace` and `wprp`.

Checked on the Linux x86_64 host:

- Parser unit tests (13), `tests/registry.rs` (every probe shape: `probes!`,
  entry and return with `debug`, `serde`, collapsed `object`, `async fn`
  with `invocation` and `panicking`, sync `unwind`, a generic fn listed
  once, `auto` under `autoref`) with exact source lines; CLI unit tests
  and `tests/cli.rs`, which runs the CLI on its own test binary (SDT site
  cross-check, the same-name warning, `--json`, the scripts).
- `cargo anyprobe list` on the `attr`, `attr_async` and `same_name`
  examples, release and release-lto: every probe listed with a site, and
  the warning for `same_name:new__entry`'s three layouts.
- Cross-target: the record sections reach codegen on every target
  (`build --lib`); the COFF objects carry `/INCLUDE:` for each record
  and the `.aprobe$a` / `$c` markers; the Mach-O objects have
  `__DATA,__anyprobe`.
- Each record is about 150 bytes (`attr`: 991 bytes for 8 probes), most of
  it the file and module paths.

Found on Linux by `attach-linux-attr.sh` (2026-10-03): in the release
build of `attr_async`, no USDT probe fired under `bpftrace -c`; release-lto
and `bpftrace -p` were fine. The kernel raises a semaphore (uprobe
`ref_ctr_offset`) by file offset, in the first writable VMA mapping that
file page. rust-lld writes the RELRO segment and the data segment back to
back in the file, and `.probes` landed in the file page that ends the RELRO
segment, which is mapped a second time at another address: the kernel
raised that copy. The registry sections only moved the layout; the release
`work` example and the spike had the same layout before phase 4. Fix: each
SDT site emits, once per object, a retained comdat byte in `.probes`
aligned to 4 KiB (x86-64) or 64 KiB (AArch64), which aligns the output
section. Applied to the spike too. `attach-linux-attr.sh` now checks
without root that exactly one writable segment maps the `.probes` page;
the unfixed spike fails that check.

After the fix, on Linux x86_64 (kernel 6.8, bpftrace 0.20.2, perf
6.8.12), release and release-lto: `attach-linux-attr.sh` passes every
check, including the bpftrace scripts `cargo anyprobe` wrote for `attr`
and `attr_async`, run under `bpftrace -c` for 20 iterations with exact
counts (every encoding decoded; 40 `fetch` entries and returns with
non-zero invocation ids; 20 `slow` unwinds with `panicking=0` and no
return; `may_panic` 20 / 10 / 10; `exported` 20 and 20).
`attach-linux.sh` passes for the spike and the `work` example, and
`attach-linux-perf.sh` for the spike.

Checked on macOS 27 arm64 (Apple M1, rustc 1.99.0, 2026-10-03):

- `list()` in a linked binary (`tests/registry.rs`, debug and release)
  through `section$start` / `section$end`, and the CLI tests on the test
  executable with the DOF cross-check (every probe has a site).
- `cargo anyprobe list` on the release and release-lto `attr`,
  `attr_async` and `same_name` examples: 8, 10 and 2 probes, each with a
  site, and the warning for `new__entry`'s three layouts. The
  `x86_64-apple-darwin` build passes the same checks without root.
- `attach-macos-attr.sh`, release and release-lto: every check passes,
  including the D scripts `cargo anyprobe dtrace` wrote, under
  `sudo dtrace -c` for 20 iterations with exact counts (every encoding
  decoded; 40 `fetch` entries and returns with non-zero invocation ids; 20
  `slow` unwinds with `panicking=0` and no return; `may_panic` 20 / 10 /
  10; `exported` 20 and 20). `attach-macos.sh` passes for the spike and
  the `work` example.
- Found on the host: `cargo anyprobe list` failed on the release `attr`
  example with "bad DOF ... misaligned". The `dof` crate reads the DOF
  headers in place with `zerocopy`, which needs 8-byte alignment, and ld64
  aligns a `__dof_*` section to one byte; that section began at an odd
  file offset. `attr_async` and the test executable happened to be
  aligned. Fix: `read_dof` copies the section to an aligned buffer first;
  a unit test places a serialized DOF section at an odd address, on every
  host.

Checked on Windows 11 x86_64 (MSVC, 2026-10-03):

- `list()` in a linked binary (`tests/registry.rs`) and the CLI tests on
  the test executable, which read the `.aprobe$b` section back through
  `/OPT:REF`.
- `cargo anyprobe list` on the release and release-lto `attr` and
  `attr_async` examples: 8 and 10 probes, all listed.
- `attach-windows-attr.ps1` (elevated), release and release-lto: every
  check passes, including `wpr -start` with the profile `cargo anyprobe
  wprp` wrote, which records exact counts (40 `fetch` entries and returns,
  20 `slow` unwinds and no return, `may_panic` 20 / 10 / 10, `exported`
  20 and 20). `attach-windows.ps1` passes for the spike and the `work`
  example.
- The script's first run failed in its own count check, not in anyprobe:
  PowerShell's comma binds tighter than `*` and `/`, and `foreach` over
  `Get-Events` iterated its wrapper array. Both fixed; the saved trace had
  the expected 200 events.

### Deferred

- `tracing` layer: forwarding spans and events to fixed anyprobe probes. It
  is independent of the registry and the CLI, and no one has asked for it.
- Listing probes from shared libraries, and `--pid` to read a running
  process.

## Phase 6: documentation from real hosts

Decided 2026-10-03, after adding `docs/usage/{linux,macos,windows}.md`,
`docs/PERFORMANCE.md`, `docs/ALTERNATIVES.md`, the new `docs/GAPS.md`
sections and the `demo` and `overhead` examples. Only the macOS material was
captured on its host. This phase brings Linux and Windows to the same
standard: every output block in a walkthrough is pasted from a real session,
and every cost in `PERFORMANCE.md` was measured on a named machine.

### Where things stand (2026-10-03)

- macOS: captured on an Apple M1, macOS 27, dtrace "Sun D 1.19", by
  `spike/scripts/capture-docs-macos.sh` (prints everything, checks nothing).
  `docs/usage/macos.md` uses that output; `PERFORMANCE.md` has the attached
  cost (0.7 to 2.1 µs per firing; the spread follows the run-to-run
  baseline, 1.2 to 3.1 ns, which fits P-core vs E-core scheduling).
- Linux: `docs/usage/linux.md` commands follow `attach-linux-attr.sh`; the
  output is written in the format that script checks and says it was not
  captured. Unknowns in it: the `bpftrace -l` listing (not shown), the
  "Attaching N probes..." line (described, not shown), whether perf reads
  the encoded `order` argument as a string. `PERFORMANCE.md` cites the
  kernel's uprobe benchmark (about 0.56 µs on a `nop`, commit
  `d41bc48bfab2`, Linux 5.17) and says anyprobe has not measured a firing.
- Windows: `docs/usage/windows.md` commands follow `attach-windows-attr.ps1`;
  the XML excerpt shows only `Data` elements, since where tracerpt puts a
  TraceLogging event's name was not checked. Attached cost: not measured.
- Disabled cost for Linux and Windows in `PERFORMANCE.md` comes from the
  spike's benchmark (`work_outlined` vs `baseline`, Threadripper 1950X), not
  from `crates/anyprobe/benches/disabled_cost.rs`.
- `docs/GAPS.md` states some behaviour nobody here has tested (below).

### Steps

1. Linux capture script, `spike/scripts/capture-docs-linux.sh`, ported from
   the macOS one. Prints, checks nothing. Runs as the user, sudo for
   bpftrace and perf only:
   - versions: `uname -r`, `bpftrace --version`, `perf --version`, CPU
     model (`lscpu`), distro;
   - starts `demo` in the background; `bpftrace -l "usdt:$BIN:*"`;
   - the walkthrough's one-liner under `-p` with `interval:s:3 { exit(); }`
     added, so the output shows the "Attaching" line;
   - `cargo anyprobe bpftrace` output and that script under `-p` for 3 s;
   - perf: `perf probe -x $BIN -a '%sdt_demo:tick' -a
     '%sdt_demo:checkout__entry' -a '%sdt_demo:checkout__return'`,
     `perf record -p` for 3 s, `perf script`, `perf probe -d`;
   - `overhead 1000000` with no tracer, five times;
   - `overhead` under `bpftrace -c` with `@[probe] = count()`, five times,
     and once with a `printf` of every argument (output discarded);
   - `cargo bench -p anyprobe --bench disabled_cost`.
   On x86_64, and on aarch64 if a machine is available (the walkthrough
   says nobody has attached to an AArch64 build by hand).
2. Windows capture script, `spike/scripts/capture-docs-windows.ps1`,
   elevated Windows PowerShell 5.1 with `-ExecutionPolicy Bypass`:
   - versions: Windows build, CPU model;
   - starts `demo`; prints its `etw-guid`; `cargo anyprobe wprp` output;
   - `wpr -start demo.wprp -filemode`, 5 s, `wpr -stop demo.etl`,
     `tracerpt ... -of XML`; prints two whole `<Event>` elements (one
     `checkout__entry`, one `tick`) so the doc can show where the event
     name appears, then the field values of every event;
   - the logman variant, and the file name logman actually writes;
   - `overhead 1000000` with no session, five times, then five times with
     the generated `overhead.wprp` session running; the event count from
     the trace, to confirm every firing was recorded;
   - `cargo bench -p anyprobe --bench disabled_cost`.
3. Update the docs from the captures:
   - `docs/usage/linux.md`, `docs/usage/windows.md`: paste real output;
     replace "was not captured" with the OS, versions and date, as
     `macos.md` does; fix any command that needed changing.
   - `docs/PERFORMANCE.md`: attached cost per firing for Linux and Windows
     with the machine; disabled-cost table from anyprobe's own benchmark
     on all three hosts; drop "not measured" wording that no longer holds.
   - `README.md` caveat: "about a microsecond on Linux and macOS" checked
     against the Linux number.
   - `CLAUDE.md`: list the three capture scripts under "Commands".
4. Check the `docs/GAPS.md` claims that are stated without a test here,
   then keep, reword or move each one:
   - the kernel removes the breakpoints and lowers the semaphore when the
     tracer is killed (`kill -9` bpftrace attached to `demo`, then check
     the semaphore through `/proc/PID/mem` at the address `readelf -n`
     gives, and that the `overhead` timings return to baseline); the same
     for dtrace on macOS;
   - a probe in a `cdylib` (`.so`, `.dylib`, `.dll`) loaded by a program
     can be traced: a small scratch crate, one probe, each OS; on macOS
     this also checks that dyld registers the library's DOF;
   - Windows: an event over 64 KB is dropped (a `&str` of 70,000 bytes),
     and a string field is cut at 65,535 bytes.
5. macOS follow-ups: run `capture-docs-macos.sh` three times and report the
   median and range per row; run the `cdylib` and kill checks from step 4.

### Done when

- Each walkthrough says which OS, tracer version and date its output came
  from, and every output block in it was pasted from a session.
- `PERFORMANCE.md` has a measured attached cost for Linux, macOS and
  Windows, each with its machine, and the disabled cost from
  `crates/anyprobe/benches/disabled_cost.rs` on each.
- Every `GAPS.md` statement about runtime behaviour has been seen on a host
  or says that it has not.
- The capture scripts are in `spike/scripts/` and listed in `CLAUDE.md`.

## FreeBSD (deferred)

Deferred indefinitely on 2026-10-02: not needed for an initial release, and
no FreeBSD machine is available. FreeBSD builds get the no-op backend, so
crates using anyprobe still compile there. The spike keeps the prototype:
runtime DOF registration through `/dev/dtrace/helper` (`spike/src/freebsd.rs`,
record parsing tested on every host) and `spike/scripts/attach-freebsd.sh`.
Its open questions, if the work resumes:

- whether `DTRACEHIOC_ADDDOF` registration works from Rust;
- whether `fasttrap` emulates the `xor eax, eax` is-enabled site;
- whether an unprivileged process may open `/dev/dtrace/helper`;
- whether `dtrace -Z -c` picks up probes registered after startup, which
  decides between lazy registration and a static constructor.

CI runs `spike-freebsd` only on manual dispatch, as information.

## Phase 0 results

The spike lives in `spike/` (crate `anyprobe-spike`, not published). It
hand-writes two probes, `spike:work__entry(u64 id, char *label, u64 len)` and
`spike:work__return(u64 id, u64 result)`, for every backend. Enabled checks
are inlined into every caller; firing happens in `#[cold]` helpers, one of
them generic. `spike/scripts/attach-{linux,macos,freebsd}.sh` and
`attach-windows.ps1` attach the native tracer and print ok/FAIL per check;
`.github/workflows/ci.yml` runs the same scripts.

### Settled locally (macOS host, 2026-10-02)

| Question | Result | How it was checked |
|---|---|---|
| macOS symbols without `dtrace -h` at build time | Yes. `__dtrace_probe$prov$name$v1$<hex C type>$...`, `__dtrace_isenabled$prov$name$v1`, fixed stability and typedefs symbols; generated directly | `dtrace -h` output compared; ld64 builds `__TEXT,__dof_spike` in dev, release and release-lto |
| ld64 patches is-enabled sites | Yes, to `mov x0, #0` (arm64); no `___dtrace_` call survives | `otool -tV` |
| Sites survive inlining, monomorphization and fat LTO | Yes on macOS: release-lto keeps 4 is-enabled sites in `main` (two inlined `work` calls plus two generic instantiations) and one probe site per cold helper | `examples/inspect_dof.rs` reading raw DOF |
| SDT notes survive linking, `--gc-sections` and fat LTO | Yes: 3 `work__entry` notes, 1 `work__return`, each PC a `nop`, semaphore addresses in `.probes`, `.stapsdt.base` retained | static `x86_64-unknown-linux-musl` link with `rust-lld`, decoded with `llvm-readobj` + `llvm-objdump` |
| SDT argument strings | Correct: `8@%rdi 8@%rsi 8@%rdx` under `att_syntax` on x86-64 | same |
| FreeBSD records emitted and retained | Yes: `set_anyprobe_probes` has `WAR` flags (`SHF_GNU_RETAIN`), one record per site | `--emit=obj` for `x86_64-unknown-freebsd`, release-lto |
| Record parsing and DOF grouping | 11 unit tests pass on every host | `cargo test` |
| `asm!` codegen for every target | Builds for x86_64/aarch64 Linux, x86_64 FreeBSD, x86_64 Windows | `cargo build --lib --target` |
| Disabled cost on macOS (Apple Silicon) | 1.276 ns probed against 0.958 ns baseline: about 0.32 ns for two probes, 0.16 ns each | criterion, `work_outlined` vs `baseline` |
| Does the `x30` clobber on macOS is-enabled sites cost anything | No. The frame is already needed for the cold calls; removing the clobber changed nothing | criterion and `otool` with and without |
| DTrace attaches on macOS with SIP on | Yes, for this (ad-hoc signed, not hardened) binary: `sudo dtrace -c` flipped the is-enabled check and read every string argument. After the tail-call fix, 40 iterations give exactly 40 firings per label, each attributed to the right function, and 160 `work-return` | run by hand on Apple Silicon, macOS 27 |
| Every macOS site rewritten and inside its function, both architectures | Yes for arm64 and x86_64, release and release-lto | `examples/inspect_dof.rs` checks each site's bytes and its function bounds |

### Settled locally (Linux x86_64 host, 2026-10-02)

| Question | Result | How it was checked |
|---|---|---|
| SDT notes in a default dynamic PIE glibc build | Yes, release and release-lto: 3 `work__entry` notes (one per cold helper instantiation), 1 `work__return`, each location a `nop`, semaphores in `.probes`, `.stapsdt.base` present | `readelf -n`, `objdump` at each location |
| Disabled cost on Linux x86_64 (Threadripper 1950X) | 2.40 ns probed against 1.63 ns baseline: about 0.77 ns for two probes, 0.39 ns each. Was 0.98 ns before the semaphore load moved into `asm!` (below). Same under release-lto | criterion, `work_outlined` vs `baseline` |
| What the remaining cost is | The register-save prologue, not the checks. One check alone measures identical to baseline: LLVM shrink-wraps the saves into the cold block. With two checks the entry cold block rejoins the hot path, so the saves stay in the entry of every call. Making the return helper a tail call (helper returns the value) did not change that | scratch criterion bench with `entry_only` / `return_only` variants; `objdump` |
| aarch64 semaphore load assembles and links directly | Yes: `adrp` + `ldrh` with `ADR_PREL_PG_HI21` / `LDST16_ABS_LO12_NC`, no GOT relocation | `--emit=obj` for `aarch64-unknown-linux-gnu`, `readelf -r` |
| bpftrace `-p` attach on x86_64 | Yes, release and release-lto (bpftrace 0.20.2, kernel 6.8): attaching raised the semaphore inside the process and detaching cleared it; `str(arg1, arg2)` read every label; across about 150 consecutive iterations each label fired exactly once per iteration and `work__return` four times, with partial iterations only at the edges of the window | `spike/scripts/attach-linux.sh` under sudo |
| perf attach on x86_64 | Yes, release and release-lto (perf 6.8.12), after moving `.stapsdt.base` into the text segment (see below): `perf probe` added all four sites, `perf record -p` raised the semaphore and cleared it on exit, and across 150 consecutive iterations each fired `work__entry` and `work__return` 4 times. bpftrace re-checked after the move, same result as above | `spike/scripts/attach-linux-perf.sh` and `attach-linux.sh` under sudo |
| SystemTap attach on x86_64 | Only with the binary linked so file offsets equal virtual addresses (`-C link-arg=-Wl,-z,separate-loadable-segments`, about 1-2% larger). Then, release and release-lto: semaphore raised and cleared, `user_string_n($arg2, $arg3)` read every label, 150 consecutive complete iterations. With rust-lld's default layout it fails (below) | `spike/scripts/attach-linux-stap.sh` with SystemTap 5.6 built from source (Ubuntu 24.04's 5.0 cannot build modules for kernel 6.8) |
| anyprobe's Linux backend (`probes!`) behaves like the spike | Yes, release and release-lto. The native glibc build of the `work` example has the spike's four SDT notes (3 `work__entry`, 1 `work__return`), each on a `nop`, with shared semaphores in `.probes` and an executable `.stapsdt.base`. bpftrace 0.20.2: attaching raised the semaphore and detaching cleared it, every label read, 149 consecutive complete iterations. perf 6.8.12: `perf probe` added the events, recording raised and cleared the semaphore, 149-150 consecutive iterations with 4 `work__entry` and 4 `work__return` each. Kernel 6.8 | `ATTACH_CRATE=anyprobe`, `attach-linux.sh` and `attach-linux-perf.sh`; `readelf -n` and `objdump` for the notes, 2026-10-02 |

### Settled locally (Windows x86_64 host, 2026-10-02)

| Question | Result | How it was checked |
|---|---|---|
| ETW session reaches the provider, events decode with `tracerpt` | Yes, release and release-lto: `logman create trace` against the printed provider GUID turned `work__entry` on in the running process and off again on stop; the decoded trace held 150 (151 for release-lto) events for each of `first-site`, `second-site`, `u32`, `u64`, equally often, plus `work__return` events | `spike/scripts/attach-windows.ps1` from an elevated prompt (`logman` needs administrator rights), Windows PowerShell 5.1 with `-ExecutionPolicy Bypass` since `pwsh` was not installed |
| anyprobe's Windows backend (lazy registration, enable callback, `tracelogging_dynamic` events) behaves like the spike | Yes, release and release-lto: `logman` session on the printed GUID turned `work__entry` on and off in the running process; the decoded trace held 151 events for each of `first-site`, `second-site`, `u32`, `u64`, equally often, plus `work__return` events. `atexit` unregistration has no check of its own | `ATTACH_CRATE=anyprobe`, `attach-windows.ps1` from an elevated Windows PowerShell 5.1 prompt, 2026-10-02 |
| Disabled cost on Windows x86_64 (same Threadripper 1950X as the Linux measurement) | 2.397 ns probed against 1.914 ns baseline: about 0.48 ns for two probes, 0.24 ns each — in the same range as Linux x86_64 (0.39 ns/probe) on identical hardware | criterion, `work_outlined` vs `baseline`, `cargo bench -p anyprobe-spike --bench disabled_cost` |

### Found by the spike

- macOS probe calls must be emitted inside `asm!`, never as Rust calls. A
  Rust call that ends a function compiles to a tail call (`b`, no `ret`); ld64
  turns it into a `nop` and execution falls off the end of the function into
  the next one. The first hand-written version did this: one `u64` site fired
  four times per call, `work__return` fired spuriously, and control ran into
  an unrelated `OnceLock` initializer that happened to return. `dtrace -h`
  headers avoid it with an `asm volatile` after the call; `usdt` calls from
  `asm!`. `inspect_dof` now fails on any site that ends its function, and
  reproduces the four bad sites when pointed at the old form.
- DTrace's function component (`probefunc`) is the mangled Rust symbol,
  hash included (`_RNvCs3iL5sSnNEqz_14anyprobe_spike10fire_entry`), so it
  changes between builds. Scripts select probes by provider and probe name
  (`spike$target:::work-entry`), which is what the stable names are for;
  documentation should say not to match on the function.
- On x86-64, ld64 records a site as the address of the call's 32-bit operand,
  one byte past the opcode, and rewrites probe calls to `nop; nopl (%rax)` and
  is-enabled calls to `xor %eax, %eax` plus three `nop`s.

- Linux: a plain Rust load of the semaphore goes through the GOT (two
  dependent loads) whenever the static lives in an rlib, even under fat LTO,
  because rustc does not mark rlib statics `dso_local` in case the rlib ends
  up in a dylib. Most probed code lives in libraries, so this is the normal
  case. `sys/sdt.h` avoids it with hidden-visibility semaphores; Rust cannot
  declare visibility, so the spike reads the semaphore with an `asm!` load and
  a `sym` operand (`movzwl sym(%rip)`, `adrp`/`ldrh`), which links directly.
  Cost, checked with a scratch crate on x86_64: a Rust `dylib` containing
  the semaphore fails to link (`R_X86_64_PC32 ... recompile with -fPIC`),
  private static or not, since a `dylib` exports it. Adding `.hidden {sema}`
  to the `asm!` lets the `dylib` link, but a crate that inlines the check
  across the `dylib` boundary (`-C prefer-dynamic`) then fails with
  `undefined hidden symbol`. `cdylib`, `staticlib` and executables link and
  run with either form. Bevy's `dynamic_linking` dev feature is a real user
  of `dylib`. Resolved below.
- Probe sites that pass pointers must be `readonly`, not `nomem`. The tracer
  reads memory through them, and under `nomem` the compiler may sink or drop a
  store to a buffer whose only reader is the probe. rustc warns about it
  (`passing pointers to nomem asm block`).
- `dof::Section` keys probes by name, so it cannot represent ld64 output
  (one DOF probe entry per function containing sites). Fine for building DOF
  on FreeBSD, where the crate controls grouping; inspection reads raw DOF.
- `criterion` 0.8 builds a C dependency unconditionally, which breaks
  `cargo clippy --all-targets --target ...` for every cross target. The spike
  uses 0.5.
- perf (6.8) converts an SDT site address to a file offset using the
  `.stapsdt.base` section's offset, so the result is only right if the base
  section shares the probe sites' segment. `sys/sdt.h` emits the base as
  `"aG"` (read-only data); GNU ld gives every segment the same
  address-to-offset delta, so that never mattered. rust-lld, the default
  linker on x86_64 Linux, does not: with `"aG"` perf placed every probe
  0x1000 bytes from its `nop`. In a release build that registered a uprobe at
  an arbitrary instruction (no events, and a risk to the traced process); in
  release-lto the kernel refused it with `ENOTSUPP` (524). The semaphore
  offset was right, since perf converts that through the `.probes` section.
  bpftrace was unaffected because it converts through the program headers.
  Fix: emit `.stapsdt.base` as `"axGR"` so it lands in the text segment;
  perf's computed offsets then match `readelf` for every site in both
  profiles, each one a `nop`.
- perf only raises the semaphore if both the kernel (4.20+, uprobe
  `ref_ctr_offset`) and perf pass it. An older perf lists the probe and
  records nothing, with no error. Docs should give the minimum versions.
- SystemTap 5.6 assumes an executable segment's file offsets equal its
  virtual addresses, which GNU ld provides and lld (rust-lld, the default on
  x86_64 Linux) does not: lld puts the text segment's file offset 0x1000
  below its address. SystemTap then registers each probe at its virtual
  address as if it were a file offset (`registration error (rc -524)`, or a
  uprobe in the wrong place), and reads the build-id from the wrong address
  in memory (`Build-id mismatch`), after which it ignores the process. The
  `.stapsdt.base` move that fixed perf does not help: SystemTap does not use
  it for this. A library crate cannot change the final link, so the fix is a
  documented build setting for SystemTap users: link with
  `-C link-arg=-Wl,-z,separate-loadable-segments` (lld) or with GNU ld.
  bpftrace, perf and gdb need neither. It affects any lld-linked binary, C
  included (a `sys/sdt.h` program linked by lld has the same 0x1000 gap).
  Upstream report drafted in `docs/upstream/systemtap-lld-file-offsets.md`,
  not yet filed.

  Decided (2026-10-02): bpftrace and perf are the supported, tested Linux
  tracers; the README names them. SystemTap gets one `docs/GAPS.md` section
  (the link flag or GNU ld, why, the size cost, the upstream report) and no
  CI or regular testing, since testing it means building SystemTap from
  source on current kernels. `attach-linux-stap.sh` stays in `spike/` for
  re-checking by hand. A Cargo feature or code change cannot help: a library
  crate does not control the final link (`cargo:rustc-link-arg` from a
  dependency's build script does not reach other packages' binaries).
- bpftrace attaches uprobes one at a time, so the sites of one probe go live
  (and come off) a few milliseconds apart. Total counts over a short window
  skew by a few iterations per site even when every site is correct. The
  attach check therefore counts per iteration (arg0 is the iteration number)
  and allows partial iterations only at the ends of the window. Generated
  bpftrace scripts and docs should not suggest comparing totals across sites
  over short windows.
- Probe names: Linux notes and ETW events use `work__entry` (perf requires C
  identifiers); DTrace shows `work-entry`, which ld64 derives from the same
  `__` spelling, and FreeBSD records spell it that way directly.
- macOS: unprivileged `dtrace` refuses to start ("DTrace requires additional
  privileges"); `sudo dtrace` works with SIP on for this binary.

### Semaphore load vs Rust `dylib`

The direct `asm!` load saves about 0.1 ns per check on Linux x86_64 but
breaks `dylib` crates linked dynamically (above). Options:

1. Direct load, with `dylib` documented as unsupported.
2. Plain Rust load through the GOT everywhere: works for every crate type,
   about 0.98 ns instead of 0.77 ns for two probes on the Threadripper.
3. Direct load by default, with an opt-out cfg (e.g. `--cfg anyprobe_got`)
   for `dylib` users. A Cargo feature would not work: unification would turn
   it on for every crate in the graph, though only performance would change.

Decided (2026-10-02): option 3, cfg named `anyprobe_dylib`. The spike
implements it in `spike/src/linux.rs`; with the cfg set, every check is a GOT
load again (checked with `objdump`). The opt-out is declared in
`[workspace.lints.rust] unexpected_cfgs`; the published crate will need the
same declaration in its own manifest or a `build.rs` (`cargo::rustc-check-cfg`)
so users who set it get no warning.

### Still open (needs CI or a VM)

- Linux aarch64: bpftrace `-p` attach, semaphore, argument reads
  (`spike-linux`). x86_64 is settled locally.
- SystemTap rebuild, if the scratch build is gone: download
  `systemtap-5.6.tar.gz` from sourceware.org; needs `libdw-dev`,
  `libboost-dev` and the running kernel's headers; then
  `./configure --prefix=DIR --disable-docs --disable-refdocs
  --disable-htmldocs --disable-server --without-nss --without-avahi
  --without-dyninst --without-python2-probes --without-python3-probes
  --without-java --without-bpf && make -j && make install`, and run the
  script with `STAP=DIR/bin/stap`.
- perf on aarch64, and whether `linux-tools` installs on hosted runners.
  Candidate: an informational `attach-linux-perf.sh` step in `spike-linux`.
- macOS: whether hosted runners allow `sudo dtrace` (`spike-macos`,
  informational). Local attach is settled.
- Disabled cost on Linux aarch64 (`spike-bench`). Linux x86_64 and Windows
  x86_64 are settled locally.
- Windows ARM64: untested, same as Linux aarch64 (see Risks).

## Risks

- macOS: SIP, Apple Silicon provider gaps, hardened-runtime binaries untraceable.
- aarch64 Linux / Windows ARM64 untested.
- The `dof` crate (Apache-2.0) covers DOF serialization; macOS needs no
  `usdt` internals, since the symbol names are generated directly.
