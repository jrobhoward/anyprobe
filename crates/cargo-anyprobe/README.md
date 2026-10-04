# cargo-anyprobe

Lists the [anyprobe](https://crates.io/crates/anyprobe) probes in a built
binary and writes tracer scripts for them.

anyprobe's macros describe each probe in a section of the binary: provider,
name, arguments and their types, and the file and line that define it.
`cargo anyprobe` reads that section from the file, so it never runs the
program and reads binaries built for any target: ELF, Mach-O (including
universal binaries) and PE.

```text
cargo install cargo-anyprobe

cargo anyprobe list --bin myapp --release
cargo anyprobe list target/release/myapp --json
cargo anyprobe bpftrace target/release/myapp > myapp.bt      # sudo bpftrace -p PID myapp.bt
cargo anyprobe dtrace --bin myapp --release > myapp.d        # sudo dtrace -p PID -s myapp.d
cargo anyprobe wprp target/release/myapp.exe > myapp.wprp    # wpr -start myapp.wprp -filemode
```

- `list` prints each probe with its arguments and where it is defined.
  `--json` gives one object per definition, with each argument's tracer
  index (`arg0`, ...).
- `bpftrace` and `dtrace` write one clause per probe that prints the probe's
  name and each argument by name, read according to its type: integers with
  their sign, pointers in hex, strings with `str` or `copyinstr`.
- `wprp` writes a WPR profile that records every provider at level Verbose,
  enabling each by the GUID TraceLogging derives from its name.
- `--bin` and `--example` build the target with cargo first, passing
  `-p`, `--profile`, `--release`, `--target`, `--features` and the other
  build options through. `--provider` and `--probe 'fetch__*'` select
  probes.

Two warnings:

- A probe name defined by several functions with different arguments (two
  methods named `new`). DTrace gives each function its own argument types,
  so a script has to branch on `probefunc`; the generated scripts print no
  arguments for such a probe.
- On Linux, macOS and FreeBSD, a probe that has a description but no
  site, because the linker removed the function's code. `list` marks it and
  the scripts leave it out. Windows binaries have no per-site metadata, so nothing is
  checked there.

The binary has to be built with the same anyprobe version as this tool;
a description written in another format is reported, not misread. So is a
description with names anyprobe's macros never write, so a binary from
elsewhere cannot put code into a generated script. `bpftrace` needs the
binary's path to hold only ASCII letters, digits and `/._-+`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.
