# Tracing on Windows with ETW

This walkthrough runs the `demo` example in one terminal and records its
events from another with the Windows Performance Recorder (`wpr`). The
commands are the ones the Windows attach checks run
(`spike/scripts/attach-windows-attr.ps1`). The output below follows the
format those checks verify; it was not captured from a session.

The `demo` example handles one made-up order per second and fires three
probes in the provider `demo`:

| Probe | Fields | Defined by |
|---|---|---|
| `tick` | `n: u64` | `probes!` |
| `checkout__entry` | `id: u64`, `customer: &str`, `order` as `{:?}` text | `#[probe]` |
| `checkout__return` | `ret: u64`, the total in cents | `#[probe]` |

Each probe is a TraceLogging event named after the probe, in an ETW provider
named after the probe's provider. The provider's GUID is derived from its
name, as TraceLogging does for every provider.

## Requirements

- An elevated prompt for `wpr`, `logman` and `tracerpt`. All three ship with
  Windows.
- For the generated profile: `cargo install --path crates/cargo-anyprobe`
  from the repository root.

## Terminal 1: run the demo

```text
> cargo build --release -p anyprobe --example demo
> target\release\examples\demo.exe
pid=7316 backend=windows-etw
etw-guid={a61566ef-3013-54c0-3fc6-0adeaddc82db}
order 0: ana, Order { items: 1, express: false }, 1750 cents
order 1: bo, Order { items: 2, express: true }, 4000 cents
...
```

The provider registers with ETW the first time one of its probes is checked,
which is the first iteration. A session can start before or after that.

## Terminal 2: record with a generated profile

`cargo anyprobe wprp` writes a WPR profile that enables every provider in
the binary by GUID:

```text
> cargo anyprobe wprp target\release\examples\demo.exe > demo.wprp
> wpr -start demo.wprp -filemode
  (wait a few seconds)
> wpr -stop demo.etl
> tracerpt demo.etl -o demo.xml -of XML -y
```

The comment at the top of `demo.wprp` names each provider and its GUID.
`wpr -start` turns the probes on in the running demo; `wpr -stop` turns them
off and writes the trace.

## Terminal 2: record with logman

`logman` takes the GUID the demo printed:

```text
> logman create trace demo -p {a61566ef-3013-54c0-3fc6-0adeaddc82db} 0xffffffffffffffff 0xff -o demo.etl -ets
  (wait a few seconds)
> logman stop demo -ets
> dir demo*.etl
> tracerpt demo_000001.etl -o demo.xml -of XML -y
```

logman can add a sequence number to the file name; `dir` shows the name it
used.

## Reading the trace

`tracerpt` writes one `Event` element per probe firing. Each holds a `Data`
element per field, named after the argument:

```xml
<Data Name="n">2</Data>
...
<Data Name="id">2</Data>
<Data Name="customer">chen</Data>
<Data Name="order">Order { items: 3, express: false }</Data>
...
<Data Name="ret">4250</Data>
```

Windows Performance Analyzer and PerfView open the `.etl` file directly.
PerfView can also record the provider by name, `*demo`, which derives the
same GUID.

## Measuring overhead

A session started before the program sees every firing:

```text
> cargo anyprobe wprp target\release\examples\overhead.exe > overhead.wprp
> wpr -start overhead.wprp -filemode
> target\release\examples\overhead.exe 1000000
> wpr -stop overhead.etl
```

[PERFORMANCE.md](../PERFORMANCE.md) describes what the numbers mean.

## When it does not work

- "Access is denied" from `wpr` or `logman`: use an elevated prompt.
- An empty trace: check the GUID against the one the program printed, and
  that the program reached its first probe while the session ran.
- ETW drops an event larger than 64 KB, or larger than the session's buffer
  size. See [GAPS.md](../GAPS.md#large-native-strings-and-byte-slices).
