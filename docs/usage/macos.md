# Tracing on macOS with dtrace

This walkthrough runs the `demo` example in one terminal and attaches
`dtrace` to it from another. The output shown was captured on an Apple M1
with macOS 27; process ids, addresses and mangled names differ on each run.

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

- `dtrace`, which ships with macOS, run with `sudo`.
- System Integrity Protection can stay on. dtrace then prints a notice on
  every run and still attaches to binaries that are not signed with the
  hardened runtime, which includes anything `cargo build` produces.
- For the generated script: `cargo install --path crates/cargo-anyprobe`
  from the repository root.

## Terminal 1: run the demo

```text
$ cargo build --release -p anyprobe --example demo
$ target/release/examples/demo
pid=87448 backend=macos-dtrace
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
$ sudo dtrace -l -p 87448 -n 'demo$target:::'
dtrace: system integrity protection is on, some features will not be available

   ID   PROVIDER            MODULE                          FUNCTION NAME
20545  demo87448              demo      _RNvCseCu1gtOLT0U_4demo4main checkout-entry
20546  demo87448              demo _RNCINvNtCs3aHLlvrSh0M_8anyprobe6encode4textKj1_NCNvNtNvCseCu1gtOLT0U_4demo8checkout10___anyprobe10fire_entry0E0BR_ checkout-entry
20547  demo87448              demo      _RNvCseCu1gtOLT0U_4demo4main checkout-return
20548  demo87448              demo _RNvNtNvCseCu1gtOLT0U_4demo8checkout10___anyprobe11fire_return checkout-return
20549  demo87448              demo      _RNvCseCu1gtOLT0U_4demo4main tick
```

- The provider is `demo` followed by the process id. `demo$target` in a
  probe description stands for it.
- There is one row per function that holds a site of the probe. The
  compiler inlined `checkout` into `main`, so the enabled checks are in
  `main` and the probe sites in the helpers that encode the arguments.
- The function column is a mangled Rust symbol that changes between
  builds. Select probes by provider and name, never by function.

## Terminal 2: attach a one-liner

```text
$ sudo dtrace -q -p 87448 \
    -n 'demo$target:::tick { printf("tick %d\n", arg0); }
        demo$target:::checkout-entry {
          printf("checkout id=%d customer=%s order=%s\n",
                 arg0, copyinstr(arg1, arg2), copyinstr(arg3, arg4));
        }
        demo$target:::checkout-return { printf("checkout returned %d\n", arg0); }'
dtrace: system integrity protection is on, some features will not be available

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
$ sudo dtrace -p 87448 -n 'demo$target:::checkout-return { printf("%d", arg0); }'
dtrace: system integrity protection is on, some features will not be available

dtrace: description 'demo$target:::checkout-return ' matched 2 probes
CPU     ID                    FUNCTION:NAME
  4  20548 _RNvNtNvCseCu1gtOLT0U_4demo8checkout10___anyprobe11fire_return:checkout-return 5250
  2  20548 _RNvNtNvCseCu1gtOLT0U_4demo8checkout10___anyprobe11fire_return:checkout-return 1750
^C
```

"Matched 2 probes" counts the two rows `dtrace -l` showed for
`checkout-return`.

## Terminal 2: run a generated script

`cargo anyprobe dtrace` writes a D script with one clause per probe, each
argument decoded by its type:

```text
$ cargo anyprobe dtrace target/release/examples/demo > demo.d
$ cat demo.d
/*
 * Generated by cargo-anyprobe 0.9.0 for /Users/me/probe_research/target/release/examples/demo
 * Run: sudo dtrace -p PID -s THIS_FILE
 */

#pragma D option quiet
/* The longest encoded argument, 4096 bytes, and its NUL. */
#pragma D option strsize=4097

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
$ sudo dtrace -p 87448 -s demo.d
dtrace: system integrity protection is on, some features will not be available

demo:tick n=8
demo:checkout__entry id=8 customer=chen order=Order { items: 3, express: false }
demo:checkout__return ret=4250
demo:tick n=9
demo:checkout__entry id=9 customer=ana order=Order { items: 1, express: true }
demo:checkout__return ret=1250
demo:tick n=10
demo:checkout__entry id=10 customer=bo order=Order { items: 2, express: false }
demo:checkout__return ret=3000
^C
```

`--provider` and `--probe 'checkout__*'` narrow the script to some probes.
`cargo anyprobe dtrace -p anyprobe --example demo --release` builds the
example first and reads the result.

## Starting the program under dtrace

`-c` starts the program with the probes already enabled, so no firing is
missed. The program then runs as root.

```text
$ sudo dtrace -c 'target/release/examples/demo 200' -s demo.d
```

## When it does not work

- "DTrace requires additional privileges": run dtrace with `sudo`.
- A probe description that matches no probes: check the provider name and
  the process id, and that the binary was built for the host architecture.
- A binary signed with the hardened runtime cannot be traced. See
  [GAPS.md](../GAPS.md#hardened-and-protected-processes).

The cost of a probe while dtrace is attached is in
[PERFORMANCE.md](../PERFORMANCE.md).
