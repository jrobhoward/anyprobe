# anyprobe — remaining work

What is left before 1.0. Exempt from the writing-style rules in `CLAUDE.md`.
Done work is not recorded here: the design is in `ARCHITECTURE.md`, the
limitations in `GAPS.md`, the costs in `PERFORMANCE.md`, and the history in
git. Delete this file once every section below is done or dropped, and
remove the references to it from `CLAUDE.md` and `ARCHITECTURE.md`.

## 1. Check what GAPS.md states without a test

Each item is a claim already in the docs. Run it, then keep, reword or move
the claim.

- [ ] A killed tracer leaves nothing behind. Linux: `kill -9` a bpftrace
      attached to `demo`; read the semaphore through `/proc/PID/mem` at the
      address `readelf -n` gives, and check that `overhead` timings return to
      baseline. The same for dtrace on macOS and FreeBSD (the is-enabled
      sites read false again). GAPS.md, "What a tracer writes into the
      process".
- [ ] A probe in a `cdylib` loaded with `dlopen` / `LoadLibrary` can be
      traced on Linux (`.so`), macOS (`.dylib`; also checks that dyld
      registers the library's DOF) and Windows (`.dll`). FreeBSD is done.
      GAPS.md, "Shared libraries".
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
