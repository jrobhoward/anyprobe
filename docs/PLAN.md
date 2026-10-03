# anyprobe — remaining work

What is left before 1.0. Exempt from the writing-style rules in `CLAUDE.md`.
Done work is not recorded here: the design is in `ARCHITECTURE.md`, the
limitations in `GAPS.md`, the costs in `PERFORMANCE.md`, and the history in
git. Delete this file once every section below is done or dropped, and
remove the references to it from `CLAUDE.md` and `ARCHITECTURE.md`.

## Where to pick up

Linux and FreeBSD (VM) have run everything in section 1
they can. Next is macOS, then Windows. On each host, in order:

### macOS

1. Re-run the attach checks. The FreeBSD commit (`e0b81e8`) added
   `dtrace_name`, `function` and `c_types` to every `define_probe!` input,
   and the macOS backend's pattern changed with it; nothing has attached on
   macOS since. As yourself (they call sudo for dtrace only):
   `spike/scripts/attach-macos.sh`, `ATTACH_CRATE=anyprobe
   spike/scripts/attach-macos.sh`, `spike/scripts/attach-macos-attr.sh`.
   `inspect_dof` runs inside them.
2. Write `spike/scripts/check-gaps-macos.sh` (bash is fine on macOS), ported
   from `check-gaps-freebsd.sh` and the cdylib half of
   `check-gaps-linux.sh`, and run it:
   - killed tracer: dtrace on `spike<PID>:::work-entry` (no `-p`), then
     `kill -9`; the `work` example must report the probe off and every
     site's bytes must match what they were before attaching. Do not look
     for `0xcc`: on arm64 the trap is a `brk`, so compare the bytes before,
     during and after. Site addresses: `spike/examples/inspect_dof.rs`
     already finds every site; reuse its output or code. Read the bytes with
     lldb (`memory read`), as the FreeBSD script does.
   - killed `dtrace -p`: does the program die with it, as on FreeBSD? Record
     either answer in GAPS.md.
   - cdylib: the scratch `plug` (providers `plugonly` and `shared`) and
     `host` (`shared`) from `check-gaps-linux.sh`, built as a `.dylib`.
     `dtrace -l -n 'plugonly<PID>:::'` must list module `libplug.dylib`
     (dyld registered the library's DOF), and `dtrace -p` must read
     `plugonly$target:::tick`. Then `shared$target:::tick`, defined in both
     files: report the firings per `probemod`. bpftrace cannot attach to
     that case on Linux.
3. Run `spike/scripts/capture-docs-macos.sh` three times; put the median
   and range per row in `PERFORMANCE.md` (section 2).
4. Update GAPS.md ("What a tracer writes into the process", "Shared
   libraries"), add the script to `CLAUDE.md` next to the other
   `check-gaps-*` scripts, and remove the macOS parts below.

### Windows

1. Re-run the attach checks, for the same macro change: elevated Windows
   PowerShell, `attach-windows.ps1` (spike), with `$env:ATTACH_CRATE =
   'anyprobe'`, and `attach-windows-attr.ps1`, each with
   `-ExecutionPolicy Bypass`.
2. Write `spike/scripts/check-gaps-windows.ps1` and run it elevated:
   - cdylib: the same scratch `plug` and `host` as a `.dll` loaded with
     `LoadLibraryW`. A `logman` or `wpr` session on the `plugonly` and
     `shared` GUIDs (`cargo anyprobe wprp` prints them) must record
     `plugonly`'s events from the DLL, and `shared` events from both the
     executable and the DLL. Each module has its own copy of anyprobe's
     provider table, so `shared` is two ETW registrations of one GUID in
     one process; check that both are enabled and both reach the trace.
   - limits: a scratch program fires a native `&str` of 70,000 bytes (the
     event should be dropped, with no error in the program) and one of
     65,600 bytes (the field should be cut at 65,535 bytes); decode with
     `tracerpt` and check each.
3. Update GAPS.md ("Shared libraries", "Large native strings and byte
   slices"), add the script to `CLAUDE.md`, and remove the Windows parts
   below.

## 1. Check what GAPS.md states without a test

Each item is a claim already in the docs. Run it, then keep, reword or move
the claim.

- [ ] A killed tracer leaves nothing behind, on macOS: `kill -9` a dtrace
      attached to the `work` example and check that every site is restored
      and the program reports the probe off. Linux and FreeBSD are done
      (`check-gaps-linux.sh`, `check-gaps-freebsd.sh`); port the FreeBSD
      script. Also check whether killing `dtrace -p` kills the program, as
      it does on FreeBSD. GAPS.md, "What a tracer writes into the process".
- [ ] A probe in a `cdylib` loaded with `dlopen` / `LoadLibrary` can be
      traced on macOS (`.dylib`; also checks that dyld registers the
      library's DOF) and Windows (`.dll`). Linux and FreeBSD are done; on
      macOS, include one probe name defined in both the executable and the
      library, which bpftrace cannot attach to on Linux. GAPS.md, "Shared
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
- [ ] macOS costs: run `capture-docs-macos.sh` three times and give the
      median and range per row in `PERFORMANCE.md` (one run so far, with a
      baseline that moved between 1.2 and 3.1 ns).

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
