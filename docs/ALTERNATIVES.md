# Alternatives

Other ways to get trace points out of a Rust program, and how they differ from
anyprobe. The descriptions of other crates reflect their sources and
documentation as of October 2026; check each project for changes.

| | anyprobe | `usdt` | `probe` | `tracelogging` | `tracing` |
|---|---|---|---|---|---|
| Linux | SDT notes with semaphores | SDT notes with semaphores | SDT notes, Android too | no (`eventheader` covers Linux `user_events`) | in-process |
| macOS | DTrace USDT, built by the linker | DTrace USDT, built by the linker | no | no | in-process |
| Windows | ETW TraceLogging | no | no | ETW TraceLogging | in-process |
| illumos, FreeBSD | no | DTrace USDT, registered at startup | no | no | in-process |
| Who reads the events | a tracer outside the process | a tracer outside the process | a tracer outside the process | an ETW session | a subscriber inside the process |
| Probes on a function's entry and return | `#[probe]` | no | no | no | `#[instrument]` spans |
| Arguments | integers, `bool`, `char`, pointers, `&str`, `&[u8]`; `serde` or `Debug` encoding | integers, pointers, strings; `Serialize` as JSON | integers, cast to `isize` | typed fields | typed fields and `Debug` |
| Arguments skipped when off | yes | yes | with `probe_lazy!` | yes | yes |
| Lists probes in a built binary | `cargo anyprobe list` | `usdt::probe_records` | no | no | no |

## `usdt`

[`usdt`](https://github.com/oxidecomputer/usdt) (Oxide Computer, Apache-2.0)
is the closest alternative. Probes are declared in a D provider file, read
by a build script or the `dtrace_provider!` macro, or as function signatures
in a module under `#[usdt::provider]`. Each probe is fired with a macro that
takes a closure, which runs only when the probe is enabled.

- It covers illumos and FreeBSD, where it builds DOF itself and registers it
  when the program calls `usdt::register_probes()`. anyprobe compiles to
  nothing on both.
- On macOS it runs the system's `dtrace -h` while the macro expands, so
  building needs `dtrace` on the host. anyprobe writes the linker symbols
  itself and builds without it, which also allows cross-compiling to macOS.
- It has no Windows backend.
- It has no attribute that probes a function's entry and return; each fire
  point is written by hand, as with anyprobe's `probes!`.
- Arguments that implement `Serialize` are passed as JSON, as anyprobe's
  `serde(..)` does. There is no `Debug` encoding.
- `usdt` takes at most six arguments per probe. anyprobe takes six values,
  where a `&str` or an encoded argument counts as two, and collapses a
  longer argument list into one JSON object.

A program that needs illumos or FreeBSD today, or already has D provider
files, fits `usdt`. One that needs Windows, or entry and return probes on many
functions, fits anyprobe.

## `probe`

[`probe`](https://github.com/cuviper/rust-libprobe) emits SystemTap SDT notes
on Linux and Android and nothing elsewhere. Every argument is cast to `isize`, so strings
and structured values have to be passed as raw pointers and read by hand.
`probe!` evaluates its arguments on every call; `probe_lazy!` skips them
when no tracer is attached.

## `tracelogging` and `tracelogging_dynamic`

Microsoft's [`tracelogging`](https://github.com/microsoft/tracelogging)
crates write ETW TraceLogging events. anyprobe's Windows backend is built on
`tracelogging_dynamic`, so the events it writes are ordinary TraceLogging
events.

The `tracelogging` macros build each event's metadata at compile time.
`tracelogging_dynamic` builds it at run time, which costs more per event
but lets anyprobe generate events from its own macros without the
`tracelogging` macros, whose expansions need `tracelogging` as a direct
dependency of every crate that uses them. A Windows-only program that wants
the lowest cost per event can use `tracelogging` directly.

## `tracing`

[`tracing`](https://github.com/tokio-rs/tracing) records spans and events
for a subscriber that runs inside the program: a logger, an exporter, a
metrics layer. What is recorded, and where it goes, is decided when the
program starts. A callsite that no subscriber wants is skipped after a
cached check.

anyprobe probes are read from outside: an operator attaches a tracer to a
running process that was not started with any tracing configured, and
detaches without restarting it. The two fit together. A program can use
`tracing` for its logs and anyprobe for points to inspect in production. A
`tracing` layer that forwards to anyprobe probes is not part of the crate.

## No instrumentation: uprobes and DTrace's `pid` provider

bpftrace (`uprobe:BIN:symbol`) and DTrace (`pid$target::symbol:entry`)
attach to any function by symbol, with no change to the program. With Rust:

- Symbols are mangled and include a hash that changes between builds.
- An inlined function has no symbol to attach to, and its callers' symbols
  change with the optimizer's choices.
- Arguments arrive as registers under the calling convention. A `&str` is
  two registers, and a struct passed by reference has to be decoded from
  memory by hand.

anyprobe's `symbol` option gives one function a stable exported name and
keeps it out of line, for tools that attach by symbol. Probes give stable
names, typed arguments and encoded values without depending on how the
compiler laid out the code.
