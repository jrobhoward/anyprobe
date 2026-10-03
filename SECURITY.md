# Security policy

## Reporting a vulnerability

Report a vulnerability by email to jrobhoward@gmail.com, or through a private
security advisory on the GitHub repository
(<https://github.com/jrobhoward/anyprobe/security/advisories/new>). Do not open
a public issue for it. A report that names the affected version, the target
and a way to reproduce it is the easiest to act on.

The crate is pre-1.0 and maintained by one person; expect an acknowledgement
within a week, not a service-level guarantee.

## Scope

In scope:

- Memory unsafety in `anyprobe`: its `unsafe` code and `asm!` blocks, the
  encoding of probe arguments, the registry parser, and the Windows provider
  runtime.
- Code that `anyprobe-macros` generates in a caller's crate and that is
  unsound, or that reads memory the caller did not pass.
- `cargo-anyprobe` panicking, hanging or running code when it reads a
  malformed or hostile binary. It reads files and never runs them.
- A dependency with a published advisory that `cargo deny` does not already
  report.

Out of scope:

- What a tracer shows. A probe passes its arguments to anyone who can attach
  a tracer to the process, which requires privilege on every supported
  platform. Arguments that hold secrets are the caller's to leave out
  (`skip(..)`).
- Tracers themselves (bpftrace, perf, SystemTap, DTrace, ETW) and the
  kernels that run them.
- Known limitations listed in [docs/GAPS.md](docs/GAPS.md).
