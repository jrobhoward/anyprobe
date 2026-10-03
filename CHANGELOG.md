# Changelog

Notable changes to `anyprobe`. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added

- `probes!`: defines probes, each a module with `enabled()` and `fire(...)`.
  Arguments are integers up to 64 bits, `bool`, raw pointers, `&str` and
  `&[u8]`, at most six values per probe. The provider defaults to the crate
  name.
- Linux (x86-64, AArch64): SystemTap SDT notes with semaphores, for bpftrace
  and perf. `--cfg anyprobe_dylib` for Rust `dylib` crates.
- macOS (x86-64, AArch64): DTrace USDT probes built by the linker.
- Windows: ETW TraceLogging events; the provider registers with ETW on the
  first enabled check of any of its probes.
- Every other target, FreeBSD included: probes compile to nothing.
