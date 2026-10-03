//! Time per call of a probed function, with or without a tracer attached.
//! `docs/PERFORMANCE.md` gives the command for each tracer.
//!
//! Usage: `overhead [CALLS]`, default 1000000.
//!
//! Each function is called `CALLS` times in a loop and the mean time per call
//! printed:
//!
//! - `baseline`: no probes.
//! - `native`: `#[probe]` with native arguments and a native return value,
//!   probes `overhead:native__entry` and `overhead:native__return`.
//! - `encoded`: `#[probe]` with a `debug` argument, probes
//!   `overhead:encoded__entry` and `overhead:encoded__return`. While a tracer
//!   is attached, each call formats the argument with `{:?}`.
//!
//! With no tracer attached, `native` and `encoded` measure the enabled checks
//! alone. Run under a tracer that enables all four probes, they measure a
//! firing of each probe on top.

use std::hint::black_box;
use std::time::Instant;

#[derive(Debug)]
struct Item {
    sku: u32,
    name: &'static str,
}

#[inline(never)]
fn baseline(id: u64, label: &str) -> u64 {
    id.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ label.len() as u64
}

#[anyprobe::probe(provider = "overhead", ret = native)]
#[inline(never)]
fn native(id: u64, label: &str) -> u64 {
    id.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ label.len() as u64
}

#[anyprobe::probe(provider = "overhead", debug(item), ret = native)]
#[inline(never)]
fn encoded(id: u64, item: &Item) -> u64 {
    id.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ u64::from(item.sku) ^ item.name.len() as u64
}

/// Mean nanoseconds per call of `f` over `calls` calls.
fn time(calls: u64, mut f: impl FnMut(u64) -> u64) -> f64 {
    let start = Instant::now();
    let mut acc = 0u64;
    for i in 0..calls {
        acc = acc.wrapping_add(f(black_box(i)));
    }
    black_box(acc);
    start.elapsed().as_nanos() as f64 / calls as f64
}

fn main() {
    let calls = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let item = Item {
        sku: 1234,
        name: "widget",
    };
    println!(
        "pid={} backend={} calls={calls}",
        std::process::id(),
        anyprobe::BACKEND
    );
    // One untimed pass of each warms caches and, on Windows, registers the
    // provider.
    for pass in 0..2 {
        let b = time(calls, |i| baseline(i, black_box("label")));
        let n = time(calls, |i| native(i, black_box("label")));
        let e = time(calls, |i| encoded(i, black_box(&item)));
        if pass == 1 {
            println!("baseline {b:>10.1} ns/call");
            println!("native   {n:>10.1} ns/call");
            println!("encoded  {e:>10.1} ns/call");
        }
    }
}
