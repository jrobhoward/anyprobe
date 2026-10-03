# anyprobe — remaining work

What is left before 1.0. Exempt from the writing-style rules in `CLAUDE.md`.
Done work is not recorded here: the design is in `ARCHITECTURE.md`, the
limitations in `GAPS.md`, the costs in `PERFORMANCE.md`, and the history in
git. Delete this file once every section below is done or dropped, and
remove the references to it from `CLAUDE.md` and `ARCHITECTURE.md`.

## Where to pick up

Linux, FreeBSD (VM) and macOS (Apple M1) have run everything in section 1
they can. Next is Windows:

### Windows

1. Re-run the attach checks. The FreeBSD commit (`e0b81e8`) added
   `dtrace_name`, `function` and `c_types` to every `define_probe!` input;
   nothing has attached on Windows since. Elevated Windows PowerShell,
   `attach-windows.ps1` (spike), with `$env:ATTACH_CRATE = 'anyprobe'`,
   and `attach-windows-attr.ps1`, each with `-ExecutionPolicy Bypass`.
2. Run `spike/scripts/check-gaps-windows.ps1` elevated, with
   `-ExecutionPolicy Bypass`. It was written on macOS and has never run;
   its scratch crates pass `cargo clippy --target x86_64-pc-windows-msvc`,
   nothing more. Expect to fix PowerShell 5.1 details on the first run.
   - cdylib: the scratch `plug` and `host` from `check-gaps-linux.sh` as a
     `.dll` loaded with `LoadLibraryW`. One `logman` session on the
     `plugonly` and `shared` GUIDs must record `plugonly`'s events from the
     DLL, and `shared` events from both the executable and the DLL. Each
     module has its own copy of anyprobe's provider table, so `shared` is
     two ETW registrations of one GUID in one process.
   - limits: a scratch program fires a native `&str` per size from 1,000 to
     70,000 bytes. `tracelogging_dynamic` cuts a counted string at 65,535
     bytes, but ETW drops an event over 64 KB including its headers, so a
     field cut at 65,535 bytes probably never reaches a trace: GAPS.md
     says the field is cut, and that is likely unobservable. The script
     checks up to 60,000 bytes arrive whole and 70,000 is dropped, and
     prints what happened to each size in between; write the largest size
     that arrives into GAPS.md.
3. Update GAPS.md ("Shared libraries", "Large native strings and byte
   slices"), add the script to `CLAUDE.md` next to the other
   `check-gaps-*` scripts, and remove the Windows parts below.

## 1. Check what GAPS.md states without a test

Each item is a claim already in the docs. Run it, then keep, reword or move
the claim.

- [ ] A probe in a `cdylib` loaded with `LoadLibrary` can be traced on
      Windows (`.dll`). Linux, FreeBSD and macOS are done. GAPS.md, "Shared
      libraries".
- [ ] Windows drops an event over 64 KB (a `&str` of 70,000 bytes) and cuts
      a string field at 65,535 bytes. GAPS.md, "Large native strings and
      byte slices".

## 2. Platforms and hosts not yet run

- [ ] The `freebsd` CI job: first run. It assumes `vmactions/freebsd-vm`
      accepts release `"15.0"`.
- [ ] Linux AArch64: bpftrace `-p` attach, semaphore, argument reads
      (`attach-linux.sh`, `attach-linux-attr.sh`), perf
      (`attach-linux-perf.sh`), the disabled cost, and the 64 KiB `.probes`
      alignment on a 64 KiB-page kernel. CI covers bpftrace on
      `ubuntu-24.04-arm`; nobody has run it by hand.
- [ ] Windows ARM64: the attach scripts and the disabled cost.
- [ ] Intel Mac: `attach-macos-attr.sh` with dtrace on an x86-64 host. The
      x86-64 build is checked without root on Apple Silicon.
- [ ] Whether hosted macOS runners allow `sudo dtrace` (the CI job reports
      it without failing).
- [ ] FreeBSD on hardware and on releases other than 15.0, to replace the VM
      costs in `PERFORMANCE.md`.
- [ ] FreeBSD AArch64: whether fasttrap supports USDT there; if so, AArch64
      site instructions in `freebsd.rs`.
- [ ] FreeBSD startup cost: time the constructor (parse, DOF build, ioctl)
      for a binary with many probes, and add it to `PERFORMANCE.md`.

## 3. Open design questions

Decide each one, or move it to GAPS.md as a known limitation.

- [ ] Crate-wide provider. A crate whose name ends in a digit (`http2`,
      `sha2`) cannot use the default provider and must repeat
      `provider = "..."` on every attribute. Options: append `_` to such
      names by default (silent; `probes!` would need the same rule); read
      `[package.metadata.anyprobe] provider` from the manifest via
      `CARGO_MANIFEST_DIR` (rebuild tracking unclear); a textually scoped
      `macro_rules!` defined at the crate root (works only for modules
      declared after it, and becomes mandatory).
- [ ] Truncation is silent. Encoded values are cut at 4096 bytes with no
      flag. A flags operand would cost one of the six values, and bpftrace's
      own 64-byte default cuts long before 4096.
- [ ] The 4096-byte cap is a constant. Making it configurable at runtime
      costs a load in the cold path only.
- [ ] Method names: two `new` methods share probe names unless one sets
      `name`. A `#[probe_impl(prefix = "Conn")]` on the `impl` block could
      supply a prefix. On FreeBSD the shared name is also indistinguishable
      by `probefunc`.
- [ ] Windows: `registration()` always returns `Ok`. ETW registration is
      lazy and retried; report a refused registration through it, or say in
      the docs that it never will.

## 4. Release

- [ ] SystemTap upstream report (rust-lld file offsets). The draft was
      removed from the tree in `e0b81e8`; recover it with
      `git show ca61f78:docs/upstream/systemtap-lld-file-offsets.md`, then
      file it or drop this item.
- [ ] First publish of `anyprobe-macros`, `anyprobe` and `cargo-anyprobe`.
      The `semver` CI job starts checking once a baseline is on crates.io.
- [ ] When this file is empty: delete it, and remove its references from
      `CLAUDE.md` (the "What this is" paragraph, the docs table row and the
      writing-style exemption) and the last sentence of `ARCHITECTURE.md`'s
      introduction.

## Not planned

Kept out unless someone asks; listed so they are not rediscovered as gaps.

- Listing probes from shared libraries loaded alongside the executable, and
  `cargo anyprobe --pid` to read a running process.
- `poll` / `pending` probes for `async fn` (would need a wrapper future).
- `#[probe]` on functions returning `impl Future` without being `async fn`
  (`#[async_trait]`).
- SystemTap in CI: it needs building from source on current kernels;
  `attach-linux-stap.sh` stays for checking by hand.
