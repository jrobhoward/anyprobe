# Tracing on Windows with ETW

This walkthrough runs the `demo` example in one terminal and records its
events from another with the Windows Performance Recorder (`wpr`). The
commands are the ones the Windows attach checks run
(`spike/scripts/attach-windows-attr.ps1`). The output below was captured on
Windows 11 Home (10.0.26100) with `spike/scripts/capture-docs-windows.ps1`;
the GUID and process ids are from that run.

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
pid=10040 backend=windows-etw
etw-guid={a61566ef-3013-54c0-3fc6-0adeaddc82db}
order 0: ana, Order { items: 1, express: false }, 1750 cents
order 1: bo, Order { items: 2, express: true }, 4000 cents
order 2: chen, Order { items: 3, express: false }, 4250 cents
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

A three-second session recorded 4 `tick`, 4 `checkout__entry` and 4
`checkout__return` events, started while the demo was already running.

The comment at the top of `demo.wprp` names each provider and its GUID.
`wpr -start` turns the probes on in the running demo; `wpr -stop` turns them
off and writes the trace.

## Terminal 2: record with logman

`logman` takes the GUID the demo printed:

```text
> logman create trace demo -p {a61566ef-3013-54c0-3fc6-0adeaddc82db} 0xffffffffffffffff 0xff -o demo.etl -ets
  (wait a few seconds)
> logman stop demo -ets
> tracerpt demo.etl -o demo.xml -of XML -y
```

`logman query providers` does not list the GUID ("Element not found"). The
provider is not registered with the system, and the session still enables it
by GUID.

## Reading the trace

`tracerpt` writes one `Event` element per probe firing. The provider name is
the probe's provider; the event ID, task and opcode are 0, and the probe name
is the event's name, shown under `RenderingInfo`. Each `Data` element holds
one field, named after the argument:

```xml
<Event xmlns="http://schemas.microsoft.com/win/2004/08/events/event">
  <System>
    <Provider Name="demo" Guid="{a61566ef-3013-54c0-3fc6-0adeaddc82db}" />
    <EventID>0</EventID>
    <Level>5</Level>
    <TimeCreated SystemTime="2026-10-03T14:29:18.912955800-05:00" />
    <Execution ProcessID="10040" ThreadID="18656" ProcessorID="1" ... />
    ...
  </System>
  <EventData>
    <Data Name="id">3</Data>
    <Data Name="customer">ana</Data>
    <Data Name="order">Order { items: 1, express: true }</Data>
  </EventData>
  <RenderingInfo Culture="en-US">
    <Task>checkout__entry</Task>
  </RenderingInfo>
</Event>
```

The other probes decode the same way: `<Data Name="n">3</Data>` for `tick`
and `<Data Name="ret">2750</Data>` for `checkout__return`. Order 3 above is
ana's express order, which the demo reported as 2750 cents.

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

The capture script ran this with 100,000 calls, which wrote 133 MB and 800,072
events with none lost. The example printed, per call with a session recording
every event:

```text
baseline        1.9 ns/call
native        864.0 ns/call
encoded      1093.2 ns/call
```

Each call fires an entry and a return probe, so a firing took about 0.4 to
0.5 µs. A session started with `logman -bs 1024 -nb 64 64` instead gave 847
ns and 1086 ns per call, with no events lost. `logman`'s default buffers are
smaller and a program this fast can overrun them; a `logman` run with the
defaults wrote less than half the data. With no session the same example printed 1.9, 2.6 and 2.4 ns.
[PERFORMANCE.md](../PERFORMANCE.md) describes what the numbers mean.

## When it does not work

- "Access is denied" from `wpr` or `logman`: use an elevated prompt.
- An empty trace: check the GUID against the one the program printed, and
  that the program reached its first probe while the session ran.
- ETW drops an event larger than 64 KB, or larger than the session's buffer
  size. See [GAPS.md](../GAPS.md#large-native-strings-and-byte-slices).
