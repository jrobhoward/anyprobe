# anyprobe — remaining work

What is left before 1.0. Exempt from the writing-style rules in `CLAUDE.md`.
Done work is not recorded here: the design is in `ARCHITECTURE.md`, the
limitations in `GAPS.md`, the costs in `PERFORMANCE.md`, and the history in
git. Delete this file once every section below is done or dropped, and
remove the references to it from `CLAUDE.md` and `ARCHITECTURE.md`.

## Where to pick up

FreeBSD (VM), macOS (Apple M1) and Windows have run everything in section 1
they can; Linux has one item left (bpftrace on `same_name`).

## 1. Check what GAPS.md states without a test

Each item is a claim already in the docs. Run it, then keep, reword or move
the claim.

- [ ] Linux: how bpftrace handles two methods named `new` that share probe
      names (the `same_name` example) — whether it attaches to both and
      reads each one's arguments. GAPS.md, "Methods with the same name",
      says it has not been checked.

## 2. Platforms and hosts not yet run

- [ ] The `freebsd` CI job: first run. It assumes `vmactions/freebsd-vm`
      accepts release `"15.0"`.
- [ ] The `windows-11-arm` jobs (build, attach, bench) in `ci.yml`: first
      run. They assume the runner has `logman`, `wpr` and an elevated
      session, as `windows-latest` does, and that `dtolnay/rust-toolchain`
      installs the aarch64 toolchain there. Read the Windows ARM64 numbers
      from the bench job into `PERFORMANCE.md`.
- [ ] Linux AArch64: read the `ubuntu-24.04-arm` attach and bench jobs for
      the disabled cost in `PERFORMANCE.md`. Perf (`attach-linux-perf.sh`) and
      a 64 KiB-page kernel stay unchecked (no hardware); GAPS.md says so.
- [ ] Whether hosted macOS runners allow `sudo dtrace` (the CI job reports
      it without failing).
- [ ] FreeBSD on hardware and on releases other than 15.0, to replace the VM
      costs in `PERFORMANCE.md`.
- [ ] FreeBSD AArch64: whether fasttrap supports USDT there; if so, AArch64
      site instructions in `freebsd.rs`.
- [ ] FreeBSD startup cost: time the constructor (parse, DOF build, ioctl)
      for a binary with many probes, and add it to `PERFORMANCE.md`.

## 3. Release

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
- The SystemTap upstream report (rust-lld file offsets). File it during a
  Linux session if convenient: the draft is at
  `git show ca61f78:docs/upstream/systemtap-lld-file-offsets.md` and needs
  its C reproducer run under `stap` first. GAPS.md documents the workaround.
- A runtime setter for the 4096-byte encoding cap, and
  `#[probe_impl(prefix = "...")]` for methods that share a name. Both are
  additive; GAPS.md has each limitation.
