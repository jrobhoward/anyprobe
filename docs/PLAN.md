# anyprobe — plan

Working plan for the pre-1.0 crate. Exempt from the writing-style rules in
`CLAUDE.md`; once the design settles it is split into `docs/ARCHITECTURE.md`
and `docs/GAPS.md` and deleted.

Cross-platform, USDT-style function probes for Rust. Annotate a function with
`#[anyprobe::probe]` and it gets stable-named entry/return probes that a tracer
can attach to in a running process. With no tracer attached the cost is one
enabled-check per probe; inlining is unaffected.

## Goals

- One attribute, four OSes: Linux, macOS, FreeBSD, Windows (no-op elsewhere).
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
| Linux | SystemTap SDT v3 notes (`.note.stapsdt`) via `asm!` | SDT semaphore (`.probes`, kernel ≥4.20 ref_ctr) | none | bpftrace, perf, SystemTap |
| macOS | DTrace USDT via linker relocations (`__dtrace_probe$…`, `__dtrace_isenabled$…`) | DTrace is-enabled | none (ld64 builds DOF) | `dtrace` (SIP: `--without dtrace`) |
| FreeBSD | DOF built from custom sections (usdt "no-linker" approach) | DTrace is-enabled | ioctl to `/dev/dtrace/helper` at startup (ctor) | `dtrace` |
| Windows | ETW TraceLogging via `tracelogging` crate; one event per probe | provider level/keyword check | `register()` at startup (ctor), unregister at exit | WPR/WPA, PerfView `*myapp`, `tracelog`, DTrace `etw` |
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
1. `anyprobe` runtime: backends, low-level `probe!` macro, no-op fallback.
2. `#[probe]` for sync fns: naming, `native`/`serde`/`debug`/`skip`, `ret`,
   compile-error diagnostics; `autoref` feature.
3. `async fn`, `unwind`, `symbol`.
4. Tooling.
5. CI matrix:
   - Linux: privileged bpftrace
   - FreeBSD: VM
   - Windows: ETW session as admin
   - macOS: metadata presence only, since SIP is on in CI

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
- Probe names: Linux notes and ETW events use `work__entry` (perf requires C
  identifiers); DTrace shows `work-entry`, which ld64 derives from the same
  `__` spelling, and FreeBSD records spell it that way directly.
- macOS: unprivileged `dtrace` refuses to start ("DTrace requires additional
  privileges"); `sudo dtrace` works with SIP on for this binary.

### Still open (needs CI or a VM)

- Linux: bpftrace `-p` attach, semaphore increments and decrements, argument
  reads, on x86_64 and aarch64 (`spike-linux`).
- FreeBSD: `DTRACEHIOC_ADDDOF` registration, `fasttrap` emulation of the
  `xor eax, eax` is-enabled site, argument reads (`spike-freebsd`).
- Windows: ETW session reaching the provider; events decoded by `tracerpt`
  (`spike-windows`).
- macOS: whether hosted runners allow `sudo dtrace` (`spike-macos`,
  informational). Local attach is settled.
- Disabled cost on Linux x86_64/aarch64 and Windows (`spike-bench`).

## Risks

- macOS: SIP, Apple Silicon provider gaps, hardened-runtime binaries untraceable.
- FreeBSD registration path is the least proven.
- aarch64 Linux / Windows ARM64 untested.
- The `dof` crate (Apache-2.0) covers DOF serialization; macOS needs no
  `usdt` internals, since the symbol names are generated directly.
