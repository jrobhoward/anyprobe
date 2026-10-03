# Tracing on FreeBSD with dtrace

This walkthrough runs the `demo` example in one terminal and attaches
`dtrace` to it from another. The output shown was captured on FreeBSD
15.0-RELEASE amd64 in a KVM virtual machine (AMD Threadripper 1950X host),
DTrace `Sun D 1.13`, on 2026-10-03; process ids and probe ids differ on each
run.

The `demo` example handles one made-up order per second and fires three
probes in the provider `demo`:

| Probe | Arguments | Defined by |
|---|---|---|
| `tick` | `n: u64` | `probes!` |
| `checkout__entry` | `id: u64`, `customer: &str`, `order` as `{:?}` text | `#[probe]` |
| `checkout__return` | `ret: u64`, the total in cents | `#[probe]` |

DTrace shows `__` in a probe name as `-`, so these are `tick`,
`checkout-entry` and `checkout-return` to dtrace.

## Requirements

- An x86-64 machine. On FreeBSD for other architectures the probes compile
  to nothing.
- DTrace loaded in the kernel before the program starts:
  `kldload dtraceall`, or `dtraceall_load="YES"` in `/boot/loader.conf`.
  The program registers its probes as it starts, and a program started
  before DTrace was loaded has none.
- A program that can open `/dev/dtrace/helper`. By default the device is
  `root:wheel`, mode `0660`, so the program has to run as root or as a member
  of `wheel`. A program running as anyone else starts normally with its
  probes off; see [When it does not work](#when-it-does-not-work).
- `dtrace`, which is part of the base system, run as root (`sudo` or
  `doas`).
- For the generated script: `cargo install --path crates/cargo-anyprobe`
  from the repository root.

## Terminal 1: run the demo

```text
$ cargo build --release -p anyprobe --example demo
$ target/release/examples/demo
pid=4600 backend=freebsd-dtrace
order 0: ana, Order { items: 1, express: false }, 1750 cents
order 1: bo, Order { items: 2, express: true }, 4000 cents
order 2: chen, Order { items: 3, express: false }, 4250 cents
...
```

The first line gives the process id for the other terminal. An optional
argument sets the interval in milliseconds: `demo 200`.

## Terminal 2: list the probes

`cargo anyprobe list` reads the probes from the binary, without root and
without running it:

```text
$ cargo anyprobe list target/release/examples/demo
demo:checkout__entry(id: u64, customer: str, order: debug)
    entry of fn checkout in demo, crates/anyprobe/examples/demo.rs:30
demo:checkout__return(ret: u64)
    return of fn checkout in demo, crates/anyprobe/examples/demo.rs:30
demo:tick(n: u64)
    probes! in demo, crates/anyprobe/examples/demo.rs:20
3 probes in 1 provider
```

dtrace lists what the running process registered:

```text
$ doas dtrace -l -n 'demo4600:::'
   ID   PROVIDER            MODULE                          FUNCTION NAME
81898   demo4600              demo                          checkout checkout-entry
81899   demo4600              demo                          checkout checkout-return
81900   demo4600              demo                                   tick
```

- The provider is `demo` followed by the process id. In a script attached
  with `-p` or `-c`, `demo$target` stands for it.
- The module is the executable or shared library that holds the probe.
- The function is the name of the function `#[probe]` annotates, and empty
  for a probe from `probes!`. Two functions that define the same probe name,
  such as `Foo::new` and `Bar::new`, are two rows that both show `new`.

## Terminal 2: attach a one-liner

```text
$ doas dtrace -q -p 4600 \
    -n 'demo$target:::tick { printf("tick %d\n", arg0); }
        demo$target:::checkout-entry {
          printf("checkout id=%d customer=%s order=%s\n",
                 arg0, copyinstr(arg1, arg2), copyinstr(arg3, arg4));
        }
        demo$target:::checkout-return { printf("checkout returned %d\n", arg0); }'
tick 2
checkout id=2 customer=chen order=Order { items: 3, express: false }
checkout returned 4250
tick 3
checkout id=3 customer=ana order=Order { items: 1, express: true }
checkout returned 2750
tick 4
checkout id=4 customer=bo order=Order { items: 2, express: false }
checkout returned 2500
^C
```

Press Ctrl-C to detach. The demo keeps running, and its probes go back to
costing one enabled check each.

A `&str` and an encoded argument each take two argument slots, a pointer and
a length, so `customer` is `arg1` and `arg2`, and `order` is `arg3` and
`arg4`. `copyinstr(ptr, len)` copies the string out of the traced process.

Without `-q`, dtrace prints its own columns before each `printf`:

```text
$ doas dtrace -p 4600 -n 'demo$target:::checkout-return { printf("%d", arg0); }'
dtrace: description 'demo$target:::checkout-return ' matched 1 probe
CPU     ID                    FUNCTION:NAME
  0  81899         checkout:checkout-return 5250
  0  81899         checkout:checkout-return 1750
^C
```

## Terminal 2: run a generated script

`cargo anyprobe dtrace` writes a D script with one clause per probe, each
argument decoded by its type:

```text
$ cargo anyprobe dtrace target/release/examples/demo > demo.d
$ cat demo.d
/*
 * Generated by cargo-anyprobe 0.1.0 for /home/me/anyprobe/target/release/examples/demo
 * Run: sudo dtrace -p PID -s THIS_FILE
 */

#pragma D option quiet
/* The longest encoded argument, 4095 bytes, and its NUL. */
#pragma D option strsize=4096

demo$target:::checkout-entry
{
	printf("demo:checkout__entry id=%u customer=%s order=%s\n", arg0, copyinstr(arg1, arg2), copyinstr(arg3, arg4));
}

demo$target:::checkout-return
{
	printf("demo:checkout__return ret=%u\n", arg0);
}

demo$target:::tick
{
	printf("demo:tick n=%u\n", arg0);
}
$ doas dtrace -p 4600 -s demo.d
demo:tick n=7
demo:checkout__entry id=7 customer=bo order=Order { items: 2, express: true }
demo:checkout__return ret=4000
demo:tick n=8
demo:checkout__entry id=8 customer=chen order=Order { items: 3, express: false }
demo:checkout__return ret=4250
demo:tick n=9
demo:checkout__entry id=9 customer=ana order=Order { items: 1, express: true }
demo:checkout__return ret=1250
^C
```

`--provider` and `--probe 'checkout__*'` narrow the script to some probes.
`cargo anyprobe dtrace -p anyprobe --example demo --release` builds the
example first and reads the result.

## Starting the program under dtrace

`-c` starts the program and enables the probes when it registers them at
startup, so no firing from `main` on is missed. The program then runs as
root.

```text
$ doas dtrace -c 'target/release/examples/demo 200' -s demo.d
```

## When it does not work

A program whose probes could not be registered runs normally with every
probe off. `anyprobe::registration()` returns the reason; the demo prints
it on its second line:

```text
$ target/release/examples/demo    # as a user outside wheel
pid=4614 backend=freebsd-dtrace
probes unavailable: cannot open /dev/dtrace/helper: Permission denied (os error 13)
order 0: ana, Order { items: 1, express: false }, 1750 cents
...
```

- "Permission denied": the program's user may not open
  `/dev/dtrace/helper`. Run it as a member of `wheel`, or add a
  `devfs.rules` entry that gives its group access to `dtrace/helper`.
- "No such file or directory": DTrace was not loaded when the program
  started. Load it with `kldload dtraceall` and restart the program.
- A probe description that matches no probes: check the provider name and
  the process id, and that `registration()` returned `Ok`.

[GAPS.md](../GAPS.md#freebsd) lists what is not supported on FreeBSD. The
cost of a probe while dtrace is attached is in
[PERFORMANCE.md](../PERFORMANCE.md).
