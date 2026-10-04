# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`anyprobe` adds USDT-style probes to Rust on Linux, macOS, FreeBSD and
Windows: stable probe names a tracer attaches to in a running process.
SystemTap SDT notes on Linux (bpftrace, perf), DTrace USDT on macOS, DTrace
USDT registered at startup on FreeBSD x86-64, ETW TraceLogging on Windows.
With no tracer attached a probe costs one enabled check, and its arguments
are not computed. Every other target compiles probes to nothing.

The crate is pre-1.0. `docs/ARCHITECTURE.md` holds the design and the
reasons behind it; read it before changing anything structural.
`docs/PLAN.md` lists only the work still planned (checks not yet run, open
design questions, release steps) and is deleted once that list is empty.

## Commands

```bash
# Build
cargo build --workspace --all-targets

# Test
cargo test --workspace
cargo test -p anyprobe-macros --test ui           # trybuild: inputs rejected under any features
cargo test -p anyprobe --test ui                  # trybuild: rejections that depend on features
cargo test some____test____name                   # single test

# trybuild cases whose expected output is rustc's own wording (a missing
# trait bound) are `#[ignore]`d, since the wording changes between rustc
# releases. They run on the toolchain pinned in ci.yml (job `ui-rustc`,
# currently 1.99.0): `rustup toolchain install 1.99.0` once.
cargo +1.99.0 test -p anyprobe-macros --test ui_rustc -- --ignored
cargo +1.99.0 test -p anyprobe --test ui_rustc -- --ignored
cargo +1.99.0 test -p anyprobe --features autoref --test ui_rustc -- --ignored

# trybuild: regenerate expected .stderr after an intended diagnostic change,
# then review the diff before committing it. The `anyprobe` cases run once
# per feature set, since each set picks different case directories. The
# `ui_rustc` cases regenerate on the pinned toolchain only, with the commands
# above and `TRYBUILD=overwrite`. Moving the pin (ci.yml and this file)
# regenerates them in the same PR.
TRYBUILD=overwrite cargo test -p anyprobe-macros --test ui
for f in "" "--no-default-features" "--features autoref"; do
  TRYBUILD=overwrite cargo test -p anyprobe --test ui $f
done

# Lint (must be clean before any change is considered done). CI runs the
# latest stable clippy, which adds lints with each release; a local toolchain
# that lags passes code CI rejects. `rustup update stable` first.
cargo clippy --workspace --all-targets -- -Dwarnings
cargo fmt --all -- --check

# Rustdoc, with broken intra-doc links treated as errors. Set on this command
# only, never in a shared `env:` block — see "Never set RUSTFLAGS" below.
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

# Cross-compile checks for the other OS backends. Each needs `--all-targets`:
# without it the `*_tests.rs` files are not compiled, and a `cfg`-gated test
# referring to something that has been renamed sails straight through. Needs
# `rustup target add` for each target once.
# FreeBSD x86-64 has its own backend; the site table tests run on every host.
for t in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu aarch64-apple-darwin \
         x86_64-apple-darwin x86_64-unknown-freebsd x86_64-pc-windows-msvc \
         aarch64-pc-windows-msvc; do
  cargo clippy --workspace --target $t --all-targets -- -Dwarnings || break
done

# `check` and `clippy` stop before codegen, so they never assemble an `asm!`
# template. A bad directive or section name in a backend only fails at codegen.
# `anyprobe` alone defines no probes, so its library build assembles nothing;
# `anyprobe-check` defines one probe per argument kind. `build --lib` reaches
# codegen and needs no linker, so it runs for every target from any host.
for t in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu aarch64-apple-darwin \
         x86_64-apple-darwin x86_64-unknown-freebsd x86_64-pc-windows-msvc \
         aarch64-pc-windows-msvc; do
  cargo build -p anyprobe-check -p anyprobe-spike --lib --release --target $t || break
done

# Feature combinations. A `cfg` gated on a feature is only compiled in the
# sets that enable it, so a default build proves nothing about the others.
for f in "" "--no-default-features" "--features autoref" \
         "--no-default-features --features autoref" "--all-features"; do
  cargo clippy --workspace --all-targets $f -- -Dwarnings || break
done
cargo test --workspace --no-default-features
cargo test --workspace --features autoref

# `--cfg anyprobe_dylib` (Linux only) replaces the direct `asm!` semaphore
# load with a plain Rust load, for Rust `dylib` crates. A separate target dir
# keeps the RUSTFLAGS change from rebuilding the default one.
for t in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
  RUSTFLAGS="--cfg anyprobe_dylib" cargo clippy --workspace --target $t \
    --all-targets --target-dir target/cfg-dylib -- -Dwarnings || break
done

# Attach checks, one per OS: build, attach the native tracer, and print
# ok/FAIL per check. The CI jobs run the same scripts. They check the spike by
# default; ATTACH_CRATE=anyprobe checks the anyprobe crate's `work` example,
# which has the same probes. Linux and macOS ask for sudo, FreeBSD for sudo
# or doas; Windows needs an elevated prompt. For the macOS scripts,
# attach-linux-attr.sh and attach-freebsd-attr.sh, SPIKE_ATTACH=0 runs only
# the checks that need no root; on macOS, SPIKE_TARGET=x86_64-apple-darwin
# checks an Intel build. The FreeBSD scripts are POSIX sh: FreeBSD has no
# bash by default.
spike/scripts/attach-linux.sh            # bpftrace
spike/scripts/attach-linux-attr.sh       # bpftrace on `#[probe]`: encodings, async, unwind, symbol, cargo anyprobe
spike/scripts/attach-linux-perf.sh       # perf probe + perf record (not in CI)
# SystemTap (not in CI) needs file offsets equal to addresses, which rust-lld
# does not produce by default; STAP picks a stap newer than the distro's.
RUSTFLAGS="-C link-arg=-Wl,-z,separate-loadable-segments" \
  CARGO_TARGET_DIR=target/sep-seg spike/scripts/attach-linux-stap.sh
spike/scripts/attach-macos.sh            # inspect_dof checks + sudo dtrace -c
spike/scripts/attach-macos-attr.sh       # `#[probe]`: encodings, same-name, async, unwind, symbol, cargo anyprobe
sh spike/scripts/attach-freebsd.sh       # dtrace -p, plus -Z -c as information
sh spike/scripts/attach-freebsd-attr.sh  # `#[probe]`: encodings, same-name, async, unwind, symbol, cargo anyprobe
# Windows: elevated Windows PowerShell. Execution policy blocks unsigned
# scripts by default, so pass Bypass for this one invocation (`pwsh` is not
# installed on most machines). Expect ok per check and a final PASS. Each
# profile header names what was checked: `== release (anyprobe-spike)` or
# `== release (anyprobe example work)`. The Linux and macOS scripts print the
# same.
powershell -ExecutionPolicy Bypass -File spike\scripts\attach-windows.ps1   # logman + tracerpt
$env:ATTACH_CRATE = 'anyprobe'
powershell -ExecutionPolicy Bypass -File spike\scripts\attach-windows.ps1   # the `work` example
powershell -ExecutionPolicy Bypass -File spike\scripts\attach-windows-attr.ps1   # `#[probe]`, every encoding, cargo anyprobe + wpr

# Doc captures, one per OS: print (and check nothing) the output pasted into
# docs/usage/<os>.md and the attached-cost numbers in docs/PERFORMANCE.md.
# Rerun after changing a probe's names, arguments or output format. Run as
# yourself; they call sudo (or need an elevated prompt) for the tracer only.
spike/scripts/capture-docs-linux.sh      # demo + overhead under bpftrace and perf
spike/scripts/capture-docs-macos.sh
sh spike/scripts/capture-docs-freebsd.sh
powershell -ExecutionPolicy Bypass -File spike\scripts\capture-docs-windows.ps1

# Checks of runtime claims in docs/GAPS.md that the attach scripts do not
# cover: a killed tracer leaves no breakpoint or raised semaphore behind,
# a probe in a dlopen'd cdylib (a DLL on Windows) can be traced, and, on
# Windows, which native string sizes ETW keeps. Not in CI.
spike/scripts/check-gaps-linux.sh        # sudo
spike/scripts/check-gaps-macos.sh        # sudo for dtrace and lldb; run as yourself
sh spike/scripts/check-gaps-freebsd.sh   # sudo or doas, lldb
powershell -ExecutionPolicy Bypass -File spike\scripts\check-gaps-windows.ps1   # elevated; DLL tracing, large strings

# Disabled-probe cost (criterion), as the CI `spike bench` job runs it
cargo bench -p anyprobe --bench disabled_cost
cargo bench -p anyprobe-spike --bench disabled_cost

# Packaging, as the CI `package` job runs it. The verify step builds the
# packaged crates as registry crates, and cargo assumes a registry crate of a
# given version never changes: a build left in `target/` by an earlier dry run
# of the same version is reused, and verification fails on code that builds
# (e.g. "no `probe` in the root"). CI starts clean; locally,
# `cargo clean -p anyprobe-macros -p anyprobe` first.
cargo publish --locked --dry-run -p anyprobe-macros -p anyprobe -p cargo-anyprobe

# Supply chain — run before adding or updating any dependency. `advisories`
# also runs weekly in CI, since the database changes with no commit here.
cargo deny check licenses bans sources advisories

# MSRV — pin to the exact floor `rust-version` in Cargo.toml declares. Needs
# `rustup toolchain install 1.88.0` once; `cargo clippy --all-targets` alone
# uses whatever toolchain is active and will not catch API usage newer than
# the floor.
cargo +1.88.0 check --workspace --all-targets
```

## Architecture

See `docs/ARCHITECTURE.md` for the design. Summary a contributor needs day to day:

- Cargo workspace, edition 2024, `rust-version = 1.88.0`.
- `crates/anyprobe-macros`: the `probes!` proc macro (`probes.rs`) and the
  `#[probe]` attribute (`attr.rs`). Target-independent: they validate the
  input, compute every string each backend needs (SDT argument format,
  DTrace symbol names, ETW field calls, register assignments) and emit one
  `::anyprobe::__private::define_probe!` per probe. `#[probe]` puts its two
  probe modules and their `#[cold]` helpers inside the function body, and
  runs the body through `__private::call_once`. Compile-fail behaviour is
  pinned by trybuild tests in `crates/anyprobe-macros/tests/ui/`, and, for
  rejections that depend on `anyprobe`'s features, in
  `crates/anyprobe/tests/ui/`.
- `crates/anyprobe`: the facade and runtime. One `cfg`-selected backend module
  per platform (`linux.rs`, `macos.rs`, `freebsd.rs`, `windows.rs`,
  `noop.rs`), each defining the `macro_rules!` that `define_probe!` resolves
  to. All `asm!` and `unsafe` live here, not in proc-macro output.
  `windows.rs` also holds the ETW provider runtime, which shares one
  registration per provider name across the process. `freebsd.rs` holds the
  startup registration; `sites.rs` parses its site table and builds the DOF
  (compiled on FreeBSD x86-64 and under `test` on every host). Each backend
  provides `registration()`, which only FreeBSD can fail. `encode.rs` writes `serde`/`debug` values into a
  thread-local buffer; `native.rs` is the public `Native` trait.
  `registry.rs` holds the probe registry: the record format, the
  `register!` support the backends forward to (each backend picks the
  section), the parser and `list()`; `error.rs` its error type and
  `RegistrationError`. The
  `serde`/`autoref` compile errors are `macro_rules!` in `lib.rs`, outside
  any backend, so they fire on every target.
- `crates/cargo-anyprobe`: the `cargo anyprobe` binary. Reads the registry
  section and the tracer metadata (SDT notes, DOF, the FreeBSD site table)
  from a built ELF, Mach-O
  or PE file with `goblin` and `dof`, and writes the `list` output and the
  bpftrace, D and WPR scripts. Its integration test reads its own test
  binary, which defines probes.
- `crates/anyprobe-check` (unpublished): one probe per argument kind, so a
  library build reaches every backend's codegen.
- `spike/` (`anyprobe-spike`, unpublished): the phase-0 hand-written probes,
  the attach and capture scripts, `examples/inspect_dof.rs`, and the FreeBSD
  prototype the backend grew from.

## Platform constraints worth knowing before editing a backend

**A probe site must survive being duplicated.** Inlining, monomorphization
and LLVM's own duplication each copy an `asm!` block. Probe sites use numeric
local labels (`990:` / `990b`), as `sys/sdt.h` does, so every copy is another
site under the same probe name rather than a duplicate-symbol error. Do not
introduce a named label in a probe site.

**`.probes` starts on a page of its own.** The kernel raises a semaphore
for perf and `bpftrace -c` by its file offset, in the first writable mapping
of that file page. rust-lld packs the RELRO and data segments back to back
in the file, so without the page-aligned byte each SDT site emits once per
object, the semaphores can share a page with the RELRO segment and every
probe stays off. `bpftrace -p` writes the semaphore itself and hides the
bug. `attach-linux-attr.sh` checks the layout without root.

**Probe sites that pass pointers are `readonly`, never `nomem`.** The tracer
reads memory through the arguments at the site. Under `nomem` the compiler may
sink or drop a store to a buffer whose only reader is the probe, such as an
encoded argument written just before it. Is-enabled sites pass no pointers and
stay `nomem`.

**A probe site that is a call must be an `asm!` call, never a Rust call.**
On macOS ld64 rewrites `__dtrace_probe$...` calls in place to `nop`. A Rust
call that ends a function compiles to a tail call with no `ret` after it, so
the rewritten site falls through into whatever function follows.
`spike/examples/inspect_dof.rs` fails on any site that ends its function; run
it on a release and a release-lto build after touching the macOS backend.

**An `async fn` body keeps its unreachable typed `return`.** `#[probe]`
runs the body as an awaited `async move` block, whose output type comes from
its first `return`, not the declared return type. The generated
`if false { let __anyprobe_never: T = loop {}; return __anyprobe_never; }`
pins it, so `return`s of other types coerce as they do in the `async fn`.
It looks like dead code; removing it breaks bodies that return
`Box::new(x)` into a `Box<dyn Trait>`. `tests/probe_async.rs` covers it.

**Argument encoding only runs behind the enabled check.** On Linux that check
is the SDT semaphore; without one, a tracer sees the probe but every call pays
for encoding its arguments. Encoding lives in `#[cold]` `#[inline(never)]`
helpers so the hot path is the check and nothing else. Do not move encoding
work in front of the check, even for a "cheap" type.

**`autoref` is strictly additive.** Anything that compiles with the feature
off must encode identically with it on, because Cargo unifies features across
the dependency graph and one crate enabling it changes the build for every
other. A change that makes `autoref` alter an encoding that already compiled
without it is a bug, whatever it fixes. For the same reason `autoref` implies
`serde` (else a second crate enabling `serde` would switch autoref's choice
from `Debug` to JSON), and an omitted `ret` means no payload with or without
`autoref`.

**Workspace members depend on `anyprobe` with `default-features = false`.**
A member (or dev-dependency) that takes the defaults turns `serde` back on
for the whole workspace, and `--no-default-features` then builds nothing
without it. `anyprobe-check` mirrors the features instead. trybuild forwards
only the features of the crate under test, which is why feature-dependent
compile-fail cases live in `crates/anyprobe/tests/ui/`.

**A registry record holds no pointers.** The same bytes are read by
`list()` in the running program and by `cargo anyprobe` from a file on disk
for any target, which works only because nothing in a record needs
relocating. Each record is a `#[used]` static built by a `const fn` from a
`concat!` the proc macro writes. A change to the record layout changes
`registry::VERSION`, and the macros, the parser and `cargo-anyprobe` change
with it; `cargo-anyprobe` depends on `anyprobe` with an exact version for
that reason.

**FreeBSD site records hold pointers; registry records do not.** Each
FreeBSD site writes its address into `anyprobe_sites`, which the dynamic
linker relocates, so that table stays out of the registry section.
`cargo anyprobe` reads only the names from it. The startup constructor never
prints or panics, keeps the DOF buffer alive for the life of the object (the
kernel rejects a registration whose buffer address it already holds), and
reports failure only through `registration()`. A change to the site record
layout changes `sites::RECORD_VERSION`, the `.byte 1` in `__anyprobe_site!`,
and `cargo-anyprobe`'s reader with it.

**Exported symbols use `#[unsafe(export_name = ...)]`.** Edition 2024 rejects
the bare form in generated code. The `symbol` option refuses generic fns,
trait-impl methods and async fns at compile time, since none has a single
stable symbol.

**A compile-time guard only fires where it is compiled.** A `compile_error!`
that rejects a feature or option combination has to live in code every
platform builds, not inside the backend module it guards. A guard placed
inside `cfg`-gated code is missing on exactly the configurations that most
need it. trybuild runs on the host only, so a guard that depends on the
target is checked with `cargo check --target` on each one.

**Generated code must be clean in the caller's crate.** It is compiled under
the caller's lints, not this workspace's. It uses absolute paths
(`::anyprobe::__private::...`), `__anyprobe_`-prefixed identifiers, and the
`#[allow]`s it needs, and the test crates that exercise it run under
`cargo clippy -- -Dwarnings` like everything else.

## Licensing is a design constraint

The crate is MIT OR Apache-2.0 with no copyleft anywhere in the dependency
graph. The projects that define the probe formats are not permissive: DTrace
is CDDL, SystemTap and bpftrace are GPL. The crate implements the formats
(SDT notes, DOF, TraceLogging metadata) from their specifications and never
copies source or headers from those projects. The `usdt` crate (Apache-2.0)
may be reused or vendored with its licence notice.

`deny.toml`'s licence allow-list is permissive-only and excludes copyleft by
omission; adding an MPL/LGPL/GPL/CDDL dependency fails CI by design. The fix
is a PR that edits the allow-list and says why, never a silent `exceptions`
entry. Crates found to be incompatible go in `[bans] deny` by name.

## CI

`.github/workflows/ci.yml` carries these jobs. Keep them when editing it.

- **build:** ubuntu, macos and windows matrix: build, test, clippy and doc
  per feature set.
- **freebsd:** a FreeBSD VM (root inside it): `cargo test`, then
  `attach-freebsd.sh` for the spike and the `work` example and
  `attach-freebsd-attr.sh`. It fails the workflow like the other attach
  jobs.
- **cross-check:** every target with `--all-targets`, plus `build --lib` per
  target for the `asm!` templates.
- **probe attach:** each OS attaches with the native tool where the runner
  allows it, for both the spike and the anyprobe `work` example. Linux uses
  bpftrace under sudo and Windows an ETW session as admin. macOS runners have
  SIP on, so that job checks the probe metadata with `inspect_dof` and reports
  a `sudo dtrace` attempt without failing on it.
  A job that cannot verify fails with an explanation rather than skipping
  silently.
- **msrv:** `dtolnay/rust-toolchain@1.88.0`, `check --locked` and
  `test --locked`.
- **ui-rustc:** `dtolnay/rust-toolchain@1.99.0`, the `ui_rustc` trybuild
  tests with `-- --ignored`. The only job pinned to a specific stable.
- **licenses** and **advisories:** separate jobs. Advisories also runs on a
  weekly cron and builds once against freshly resolved dependencies
  (`cargo update`).
- **package:** `cargo publish --locked --dry-run` for `anyprobe-macros`,
  `anyprobe` and `cargo-anyprobe` together.
- **semver:** `cargo-semver-checks`, skipped with a notice until a baseline
  exists on crates.io.
- **fmt.**
- **spike bench:** `cargo bench --bench disabled_cost` for the spike and
  `anyprobe` on each OS, `continue-on-error` since shared runners are noisy.

**A multi-line `run:` in the cross-OS matrix needs `shell: bash`.** The
Windows default shell is PowerShell, which does not parse `if`, `[`, `&&` or
`2>/dev/null`. A step whose body is more than one `cargo` invocation sets the
shell explicitly; a job with more than a couple of such steps sets
`shell: bash` under `defaults:` once.

**Never set `RUSTFLAGS=-Dwarnings` in the CI environment** (or any shared
`env:` block). It applies to dependency compilation too, so a new stable rustc
that adds one warning anywhere in the graph reds every job with no anyprobe
changes. Pass `-Dwarnings` to `cargo clippy` explicitly instead, which scopes
the deny to this workspace.

## Definition of Done

Before considering any change complete:

- `cargo test --workspace` passes with zero failures, trybuild included
- The `ui_rustc` tests pass on the pinned toolchain (commands under "Test")
  after touching anything they cover
- `cargo clippy --workspace --all-targets -- -Dwarnings` is clean
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` is clean
- `cargo fmt --all -- --check` is clean
- `cargo deny check licenses bans sources advisories` passes
- `Cargo.lock` is committed, so a dependency change shows up in the diff
- The cross-compile clippy loop and the `build --lib` loop pass on every
  target
- The feature-combination commands pass. A change under a `cfg` or a
  `#[cfg(feature)]`-gated test is not done until the sets that exclude it have
  been run
- `cargo +1.88.0 check --workspace --all-targets` (MSRV) passes
- No `.unwrap()` / `.expect()` in production code, generated code included
- New public items have doc comments (`missing_docs` catches this)
- A changed diagnostic has its trybuild `.stderr` updated and reviewed
- `CHANGELOG.md` has an entry for anything a user would notice
- If behaviour changed on a platform, it was verified with real tools there
  (bpftrace, dtrace, an ETW session), or the fact that it was not is stated
  plainly. Run the attach scripts from "Commands" on each OS, once with the
  spike and once with `ATTACH_CRATE=anyprobe`: `attach-linux.sh` (sudo),
  `attach-macos.sh` (sudo), `attach-freebsd.sh` (sudo or doas) and
  `attach-windows.ps1` (elevated prompt, with `-ExecutionPolicy Bypass`).
  `#[probe]` has an attach script on Linux (`attach-linux-attr.sh`), macOS
  (`attach-macos-attr.sh`, sudo), FreeBSD (`attach-freebsd-attr.sh`) and
  Windows (`attach-windows-attr.ps1`). All four scripts cover `async fn` and
  `unwind` (the `attr_async` example); macOS and FreeBSD (`pid` provider) and
  Linux (uprobe) cover `symbol` by name, since Windows has no tracer that
  attaches by symbol. Run
  the macOS scripts as yourself, not under `sudo`: they call `sudo` for dtrace only, and cargo run as root
  leaves root-owned files in `target/`
- `cargo publish --locked --dry-run -p anyprobe-macros -p anyprobe -p cargo-anyprobe` passes

## Docs are part of "done"

Each file has one job; keep changes in the right one rather than restating
across them:

| File | Holds | Scope |
|---|---|---|
| `README.md` | What the crate does, how to use it, and the caveats that change how it should be used | Link out rather than expand |
| `docs/PLAN.md` | Work still planned before 1.0: unrun checks, open design questions, release steps | Exempt from the writing-style rules. Remove finished items rather than marking them done; a settled decision moves to `ARCHITECTURE.md` or `GAPS.md`. Deleted when empty |
| `docs/ARCHITECTURE.md` | Module map and why each backend was chosen over its alternatives | Update when a design decision changes; not a development log |
| `docs/GAPS.md` | Every known limitation, why it exists, and what changing it costs | One section per gap. Add to it rather than quietly narrowing scope |
| `docs/PERFORMANCE.md` | The cost of a probe with and without a tracer, per platform, and how it was measured | Measured numbers with the machine they came from; say plainly what has not been measured |
| `docs/ALTERNATIVES.md` | How anyprobe compares with other crates and with attaching by symbol | Facts about other projects, dated; no ranking |
| `docs/usage/*.md` | One walkthrough per OS: run the `demo` example, attach the native tracer, expected output | Say whether the output was captured on that OS. Rerun after changing a probe's names, arguments or output format |
| `CHANGELOG.md` | What changed in each release, and enough of why to act on it | Keep a Changelog format. One entry per released version |
| `SECURITY.md` | How to report a vulnerability, and what is in scope | Reporting process and scope, not a list of known issues |
| `CLAUDE.md` | Conventions and constraints a contributor needs before editing | Rules, not narrative |

## Writing style for `README.md`, `docs/**/*.md` and rustdoc

`docs/PLAN.md` is exempt while it exists.

- **No second person, no first person.** Not "your function", "you can", "we
  chose". Describe the crate and what it does: "emits an SDT note", "the
  backend registers the provider at startup". Imperatives are fine in
  instructions. The dual-licence boilerplate in the README is standard legal
  text and stays as it is.
- **Bold is for bullet lead-ins only** — the first word or phrase of a list
  item. No bold mid-sentence, none in table cells, none opening a paragraph.
  Italics are for genuine contrast, used sparingly.
- **No decorative icons.** Write "yes" and "no" in tables, not ✅ and ❌.
- **Do not sell.** Avoid "the whole point", "load-bearing", "genuinely",
  "crucially", "deliberately", "notably". State the fact and stop.
- **Plain words, short sentences.** Prefer "use" over "utilize", "about" over
  "approximately", "does nothing" over "is inert".
- **Understate the caveats.** "Nobody has run it on FreeBSD yet" beats "a
  critical unverified gap".
- **No development-phase framing.** Don't attribute a fact to "the Phase N
  spike" or narrate how a decision was reached over time. State the current
  fact and, if the reasoning matters, the reasoning.

## Conventions

**Workspace lints:** lints live in the root `Cargo.toml` under
`[workspace.lints]`, and every crate opts in with `[lints] workspace = true`.
A crate-local `[lints]` table is not added; a lint that one crate needs to
relax is relaxed with a scoped `#[allow]` and a comment.

**Test file layout:** tests live in separate `*_tests.rs` files, registered at
the bottom of the source file with:
```rust
#[cfg(test)]
#[path = "encode_tests.rs"]
mod encode_tests;
```
Integration tests live in each crate's `tests/`. Compile-fail tests live in
`crates/anyprobe-macros/tests/ui/`, one case per `.rs` file with its expected
`.stderr` beside it; cases whose result depends on `anyprobe`'s features live
in `crates/anyprobe/tests/ui/<feature-set>/`. A case whose output is mostly
rustc's wording rather than a message this repo writes goes in the matching
`tests/ui_rustc/` directory instead (run by `ui_rustc.rs`, `#[ignore]`d).

**Test naming:** `subject____condition____result` — exactly four underscores
between segments. Because consecutive underscores trip `non_snake_case`, every
`*_tests.rs` file carries `#![allow(non_snake_case)]` at the top, alongside
`#![allow(clippy::unwrap_used)]` and `#![allow(clippy::expect_used)]`. trybuild
case files use the same pattern as their file name
(`symbol____on_generic_fn____is_rejected.rs`).

**No `.unwrap()` / `.expect()` in production code** — use `?`. `clippy.toml`
allows them in tests only. Workspace lints also warn on `cognitive_complexity`,
`must_use_candidate`, `return_self_not_must_use`, `missing_errors_doc` and
`undocumented_unsafe_blocks`: a new public method that returns `Self` or a
`Result` has to carry `#[must_use]` or an `# Errors` section, and every
`unsafe` block carries a `// SAFETY:` comment. These are lint gates rather
than conventions, so none can be skipped quietly.

**Public docs:** every new public item needs a doc comment — `missing_docs` is
on, so this is enforced rather than asked for. Doc examples that use a gated
API must be `cfg`-gated too; `cargo test` runs them. `README.md`'s example is
compiled as a doctest via `#[cfg(doctest)]` in `lib.rs`, so it cannot drift.
Rustdoc must not link to a repository-relative path like `docs/GAPS.md` — a
docs.rs reader cannot follow one; use the absolute GitHub URL there and keep
relative links in the Markdown files. docs.rs builds one target per backend
(`package.metadata.docs.rs`), with `doc_cfg` behind the `docsrs` cfg so each
item shows the platform it belongs to.

**Errors:** `thiserror`, in `error.rs`. Public error enums are
`#[non_exhaustive]`.

**Public API:** pre-1.0, so a breaking change is a minor version bump with a
`CHANGELOG.md` entry. `__private` is `#[doc(hidden)]` and outside the semver
contract, but the macro and runtime crates are released in lockstep with an
exact (`=`) version requirement between them, so generated code always
matches the runtime it calls.

**Unsafe:** `unsafe_op_in_unsafe_fn` is `forbid` and
`missing_debug_implementations` is `warn`, both in `[workspace.lints]`. FFI calls, `asm!` blocks and
`ioctl`s need a `// SAFETY:` comment saying why the operation is sound — what
the arguments point at, what the callee does with them, and what is kept
alive.
