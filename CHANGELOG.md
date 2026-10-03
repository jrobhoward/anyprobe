# Changelog

Notable changes to `anyprobe`. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added

- `#[probe]`: probes a function's entry and return (`name__entry`,
  `name__return`). Options: `name`, `provider`, `serde(..)`, `debug(..)`,
  `skip(..)`, `native(..)` and `ret = native | serde | debug`. Integers,
  `bool`, `char`, raw pointers, `&str`, `&[u8]` and references to scalars
  are passed natively; any other argument must be listed. Encoded values are
  NUL-terminated strings, cut at 4096 bytes. Arguments that would take more
  than six values are passed as one JSON object.
- `Native`: lets `native(..)` pass a type alias or newtype as one 64-bit
  value.
- Features: `serde` (default) for JSON encoding; `autoref` to encode unlisted
  arguments as JSON or `{:?}` instead of rejecting them.
- `probes!` accepts `char`.
- Windows: probes share one ETW registration per provider name across the
  process, instead of one per `probes!` block.
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
