# Plan: improvements from the `usdt` comparison

Follows from [usdt-comparison.md](usdt-comparison.md), which also lists the
ideas considered and left out. Items are ordered by value. Each item ends
with its own checks. The Definition of Done in `CLAUDE.md` applies to every
item on top of those.

| # | Item | Kind | Risk | Breaking? |
|---|---|---|---|---|
| 1 | Retain SDT notes (dangling notes after `--gc-sections`) | Bug fix | Low | No |
| 2 | Correlation id for `probes!` | Feature | Low | No (additive) |
| 3 | Lazy `fire_with` closure form | Ergonomics | Medium (macro work in every backend) | No (additive) |
| 4 | Six-argument check on Apple Silicon macOS | Test | Low | No |
| 5 | Native `Option<&str>`, `Option<&[u8]>`, `&CStr` | Feature | Low to medium | No (additive) |
| 6 | Lint test for `probes!` output | Test | Low | No |
| 7 | Docs: `ALTERNATIVES.md`, `GAPS.md` | Docs | None | No |

## 1. Retain SDT notes

**Problem.** A probe in a function that `--gc-sections` removes keeps its
`.note.stapsdt` entry, and with rust-lld (the default on x86-64 Linux) the
note's location resolves to a small offset into the ELF header (`0xb` in
release, `0x42` in dev in the reproduction). GNU ld keeps the function
instead. This is usdt issue #498.

**Change.** In both `__anyprobe_sdt_site!` variants in
`crates/anyprobe/src/linux.rs`, change

```text
".pushsection .note.stapsdt, \"\", \"note\"",
```

to

```text
".pushsection .note.stapsdt, \"R\", \"note\"",
```

An experiment on a scratch copy showed this keeps the function and gives a
valid note in dev and release, as GNU ld does. `"?"` had no effect, and
`"aR"` failed to link.

**Steps.**

1. Make the flag change and extend the doc comment above the macro: why
   `"R"`, what it costs (a function whose only purpose was dead code now
   stays in the binary), and why `"o"` was not used (it needs a named
   label).
2. Add a regression fixture: a library crate (or a module in
   `anyprobe-check`) with a `pub fn` holding a probe that the test binary
   never calls, linked into a binary. `anyprobe-spike` or a new example is
   the likely home.
3. Extend the no-root layout check in `spike/scripts/attach-linux-attr.sh`:
   every `.note.stapsdt` location must fall inside an executable `PT_LOAD`
   segment (`readelf -n` plus `readelf -lW`). It must fail before the fix and
   pass after it.
4. Before the fix, record what `bpftrace -l` and an attach to the phantom
   probe do, for the CHANGELOG entry and in case the old behaviour was worse
   than a probe that never fires.
5. Check that the problem does not exist on FreeBSD. `anyprobe_sites` is
   already `"awR"`, which should keep the function alive the same way; check
   it on the FreeBSD VM with `readelf` on the same fixture, since a site
   address in the ELF header would go into the DOF.
6. Run `attach-linux.sh`, `attach-linux-attr.sh` and
   `attach-linux-perf.sh` with the spike and with `ATTACH_CRATE=anyprobe`.
   Run the cross-compile `build --lib` loop for both Linux targets.
7. Record the binary size change for the `work` example in the PR
   description; mention it in `docs/PERFORMANCE.md` only if it is not
   negligible.
8. Update the `.probes` and notes paragraph in `docs/ARCHITECTURE.md` and add
   a `CHANGELOG.md` entry under Fixed.

**Follow-up, not in scope.** `SHF_LINK_ORDER` (`"o"`) would drop the note
with the function instead. Consider it only if the kept dead code turns out
to matter.

## 2. Correlation id for `probes!`

**Problem.** `#[probe]` pairs async entries and returns with an invocation
id, but a user writing `request__start` / `request__done` with `probes!` has
nothing built in, where usdt has `UniqueId`.

**Change.** Expose the existing `__private::next_invocation()` generator
through a public function, for example:

```rust
/// A new id, unique within the process and never 0, to pass to several
/// probes so a tracer can pair them.
#[must_use]
pub fn next_id() -> u64
```

It is a `u64`, so it goes through `probes!` as a native argument with no new
argument kind. `#[probe]` keeps calling the same counter, so ids never
collide between the two.

**Steps.**

1. Add the function in `lib.rs` (keep `#[cold]` `#[inline(never)]`: it is
   only called behind an `enabled()` check). Decide the name (`next_id`,
   `correlation_id`) before writing docs.
2. Rustdoc with an example that checks `enabled()` once and passes the id to
   a start and a done probe.
3. A test in `crates/anyprobe/tests/probes.rs`: ids are non-zero and
   distinct, including across threads.
4. Add the pattern to the README's `probes!` section (short) and a
   `CHANGELOG.md` entry under Added.
5. Decide, and document, whether the id is meaningful when the start probe
   was off: a pattern that skips the id when `enabled()` was false needs the
   "0 means not traced" convention `#[probe]` already uses.

## 3. Lazy `fire_with` closure form

**Problem.** `request__start::fire(id, format_path(p))` outside an
`enabled()` check computes its arguments on every call. usdt's closure-only
API makes that impossible. Replacing `fire` would be a breaking change, so
add a form beside it.

**Change.** Each probe module gets:

```rust
#[inline(always)]
pub fn fire_with(f: impl FnOnce() -> (T0, T1, ...)) {
    if enabled() {
        let (a0, a1, ...) = f();
        fire(a0, a1, ...);
    }
}
```

**Steps.**

1. Lifetimes: `impl FnOnce() -> (u64, &str)` has no input lifetime to elide
   from, so the macro must name one (`fn fire_with<'a>(f: impl FnOnce() ->
   (u64, &'a str))`). Write this in `probes.rs` and pass the signature to
   `define_probe!`, or generate it in the `macro_rules!` of each backend.
   Prefer the proc macro so every backend gets one definition.
2. Add it to every backend: `linux.rs`, `macos.rs`, `freebsd.rs`,
   `windows.rs`, `noop.rs`. On `noop`, `fire_with` must not call `f`.
3. Zero-argument and one-argument probes: decide whether the closure
   returns `()` and `(T,)` or a bare `T`. A bare `T` for one argument is
   friendlier but is one more special case; usdt accepts both.
4. Check that the closure lands in the cold path: look at the release
   assembly for `anyprobe-check`, and add a `fire_with` case to the
   `disabled_cost` bench. The disabled cost should match
   `if enabled() { fire(..) }`.
5. trybuild cases: wrong arity, wrong types, borrowed `&str` from a local,
   all with readable errors.
6. README: show `fire_with` as the default form and keep `enabled()` for
   sharing one check across several probes. `CHANGELOG.md` under Added.
7. Attach scripts on each OS, since this touches every backend.

## 4. Six-argument check on Apple Silicon macOS

**Problem.** usdt #62 reports arg5 arriving as NULL on an M1 Mac and was
never reproduced on x86. anyprobe passes six register values on aarch64
macOS too, and no check targets the sixth register by value.

**Steps.**

1. Check whether the spike or the `work` example already fires a probe with
   six native values and checks every one under dtrace. If not, add one with
   six distinct constants.
2. Add a check to `attach-macos.sh` (and `attach-macos-attr.sh` for a
   six-value `#[probe]`) that prints `arg0`..`arg5` and compares all six.
3. Run it on an Apple Silicon machine and, with
   `SPIKE_TARGET=x86_64-apple-darwin`, on an Intel build. Do the same on
   Linux aarch64 and FreeBSD for completeness, since it is cheap.

## 5. Native `Option<&str>`, `Option<&[u8]>` and `&CStr`

**Problem.** usdt #487 and #488 ask for these; anyprobe also lacks them
natively, so today they need `debug(..)` or `serde(..)`.

**Change.**

- `Option<&str>` / `Option<&[u8]>`: two slots, `(ptr, len)`, with a null
  pointer and length 0 for `None`.
- `&CStr`: one slot, the pointer. The tracer reads it NUL-terminated, so it
  also works with perf and gdb, which read a native `&str` past its end
  (`docs/GAPS.md`, "Native `&str` has no terminator").

**Steps.**

1. `args.rs` classifies types by spelling: add the three spellings, with
   unit tests in `args_tests.rs`, and the SDT type strings, DTrace C types
   (`char *`) and ETW field calls for each.
2. Windows: decide how `None` appears in an ETW event (an empty string, or a
   separate presence field). An empty string is simplest; record the choice
   and that `None` and `Some("")` look the same in `docs/GAPS.md`.
3. `cargo anyprobe` script generation: the bpftrace and D scripts must read
   a null pointer safely (`arg1 ? str(arg1, arg2) : "(none)"` in bpftrace;
   the same guard in D).
4. Add each kind to `anyprobe-check` so `build --lib` covers every
   backend's codegen.
5. Attach scripts on each OS, README argument list, `CHANGELOG.md` under
   Added.

Do the two `Option` kinds and `&CStr` as separate PRs if the ETW decision
takes discussion.

## 6. Lint test for `probes!` output

**Problem.** usdt's expansions fail `clippy::cast_lossless` in callers'
crates (#240, #270). `#[probe]` output carries
`#[allow(clippy::all, clippy::pedantic, clippy::nursery)]`; nothing shows
whether `probes!` output, and what a user writes around `enabled()` and
`fire()`, is clean under the stricter groups.

**Steps.**

1. Add an integration test file (for example
   `crates/anyprobe/tests/probes_lints.rs`) with
   `#![warn(clippy::pedantic, clippy::nursery, clippy::cast_lossless)]` that
   defines a probe of every native kind with `probes!` and fires it.
2. The workspace `cargo clippy --all-targets -- -Dwarnings` gate then fails
   on any lint the expansion trips. Fix what it finds in the generated code
   with scoped `#[allow]`s or code changes, not by relaxing the test.
3. Run it across the cross-compile clippy loop, since each backend expands
   differently.

## 7. Documentation

**Steps.**

1. `docs/ALTERNATIVES.md`, `usdt` section, dated October 2026. Add these
   facts: `&str` is copied into a NUL-terminated buffer on every enabled
   fire; JSON values are wrapped in `{"ok": ...}`; `UniqueId` correlates
   probes; pointer arguments are limited to pointers to integers; arguments
   are given as a closure. Mention the open dangling-note issue (#498) only
   as a fact about usdt, and only while it is open. No ranking, per the
   file's rules.
2. `docs/GAPS.md`: if item 1 does not land in the same release, add a
   section on dangling notes under "Linking and dependencies": what happens,
   that GNU ld is unaffected, and the workaround
   (`-C link-arg=-Wl,--no-gc-sections` or `-fuse-ld=bfd`). Remove it when
   item 1 lands.
3. If item 2 or 3 lands, update the "Probes on a function's entry and
   return" and "Arguments" rows in the `ALTERNATIVES.md` table as needed.

## Status (2026-10-04)

All seven items are implemented in one squashed commit, `cc9a696` on branch
`usdt-followups`. Changes from the plan:

- Item 3 is `anyprobe::fire!(probe(args))`, a `macro_rules!`, not a
  `fire_with(closure)` method. A closure returning `(u64, &str)` has no
  lifetime to elide from, so every reference and hidden lifetime would need
  rewriting; the macro needs no backend changes. Disabled cost 2.17 ns
  against 2.38 ns for the hand-written check (Linux x86-64, `disabled_cost`).
- Item 5 adds registry types `opt_str`, `opt_bytes` and `cstr` instead of
  reusing `str`/`bytes`, so only optional strings get the D null guard.
- Item 7 touched `ALTERNATIVES.md` only: item 1 landed, so `GAPS.md` needs no
  dangling-note section.

Passed: the full Definition-of-Done gate on Linux x86-64 (tests, clippy on
all seven targets and five feature sets, `build --lib` per target, MSRV,
`ui_rustc` on 1.99.0, deny, package dry run); `attach-linux-attr.sh` and
`attach-linux.sh` (spike and `ATTACH_CRATE=anyprobe`) with bpftrace 0.20.2,
release and release-lto; `attach-freebsd-attr.sh` on FreeBSD 15 x86-64.

## Remaining

Pushing the branch runs most of what is left in CI: Linux AArch64 attach
(`ubuntu-24.04-arm`), Windows attach as admin on x64 and ARM64, the macOS
no-root checks (`inspect_dof`, script counts) and the FreeBSD VM job. What
CI cannot do:

### macOS: done on Apple Silicon (2026-10-04)

Run on macOS 27.0.1, arm64, SIP on. Three things came out of it, all
uncommitted on top of `cc9a696`:

- The six-value check failed: DTrace on arm64 reports a USDT probe's `arg5`
  as 0, while `uregs[R_X5]` holds the value (16) and the `pid` provider's
  `arg5` is right. The site was checked in the disassembly (`mov w5, #0x10`
  before the `nop`), so it is the kernel, as in usdt #62, not anyprobe.
  Decision: lower the limit to five values on every target (`MAX_OPERANDS`),
  while nothing is released. `probes!` rejects six; `#[probe]` collapses
  into JSON past five. The `attr` example's `six` probe is now `five`, and
  all four attr attach scripts check the fifth value on its own.
  `check-gaps-macos.sh` keeps a six-argument probe in a scratch C program
  (`dtrace -h`) and checks `arg5` still reads 0 on arm64, so the limit can
  go back to six if macOS fixes it. An interim `uregs[R_X5]` change to
  `cargo anyprobe dtrace` was reverted.
- `probes_lints` and `anyprobe-check`'s `gc_sections` failed to link on
  macOS: probes named `unsigned` and `signed` are D keywords, and ld64
  compiles a D declaration of every provider. Renamed to `unsigned_ints` and
  `signed_ints`. The macros now reject the 56 names D reserves (keywords and
  `int8_t`..`uintptr_t`) as provider or probe names on every target; each was
  confirmed to fail linking. Kernel typedefs (`size_t`, `pid_t`, ...) fail
  too but vary by release, so GAPS.md documents them instead.
- Passed: `attach-macos-attr.sh`, `attach-macos.sh` (spike and
  `ATTACH_CRATE=anyprobe`), `check-gaps-macos.sh`, each on release and
  release-lto; the no-root parts with `SPIKE_TARGET=x86_64-apple-darwin`; and
  the full Definition-of-Done gate run from macOS (tests and clippy per
  feature set, clippy and `build --lib` on all seven targets, doc, fmt,
  `ui_rustc` on 1.99.0, MSRV, deny, package dry run).

Not done: dtrace on an Intel Mac. Whether x86-64 macOS reads `arg5`
correctly is unchecked; `check-gaps-macos.sh` checks it there.

Rerun after the five-value change: Linux and FreeBSD done (below);
Windows still to do.

### Windows x64: done (2026-10-04, on `4e1593f`)

Windows 11 x64, elevated. `attach-windows-attr.ps1`, `attach-windows.ps1` for
the spike and `ATTACH_CRATE=anyprobe`, release and release-lto: all checks
ok, PASS. A kept `attr` trace holds 76 `Name="name"` events with empty
content (`None`) and 75 with `opt`. The Definition-of-Done gate passed
from Windows (tests, clippy per feature set, clippy and `build --lib` on all
seven targets, doc, fmt, `ui_rustc` on 1.99.0, MSRV, deny, package dry run).
ARM64 Windows is not run locally; CI covers it.

The section below is what was run.

### Windows (x64; ARM64 if available)

CI runs these as admin, so a push may be enough. To run locally, from an
elevated Windows PowerShell:

1. `powershell -ExecutionPolicy Bypass -File spike\scripts\attach-windows-attr.ps1`:
   the new checks are `five native values, the fifth on its own` (fields
   `neg` = -12 and `last` = 16), `Option<&str>, Option<&[u8]> and &CStr`
   (fields `name` = opt and `label` = cee), and `list: attr has 12 probes`.
2. `attach-windows.ps1`, once plain and once with
   `$env:ATTACH_CRATE = 'anyprobe'`.

Not checked by any script: that a `None` reaches ETW as an empty field, as
`docs/GAPS.md` ("`None` and an empty value look alike") states. The `attr`
example passes `None` for `name` on odd ids, so the trace already holds such
events; a check would look for `Name="name"` with empty content.

### Linux and FreeBSD: done (2026-10-04, on `9f366dc`)

- Linux x86-64, bpftrace 0.20.2 and perf, release and release-lto:
  `attach-linux-attr.sh` (94 checks), `attach-linux.sh` for the spike and
  `ATTACH_CRATE=anyprobe` (32), `attach-linux-perf.sh` (10, its first run on
  this branch: the retained notes do not change what perf sees). The full
  Definition-of-Done gate passed again (34 steps).
- FreeBSD 15 x86-64, release and release-lto: `cargo test --workspace`,
  `attach-freebsd-attr.sh`, `attach-freebsd.sh` for the spike and
  `ATTACH_CRATE=anyprobe`.

Not run locally: Linux AArch64. The CI `ubuntu-24.04-arm` attach job covers
it on push.
