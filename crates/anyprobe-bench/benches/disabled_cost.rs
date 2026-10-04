//! Cost of a probed function with no tracer attached, against the same
//! function with no probes. `probed` mirrors `spike_disabled_cost.rs`, so the
//! generated code can be compared with the hand-written probes; `attribute`
//! is the same function under `#[probe]` with the same arguments and a native
//! return value; `fire_macro` is `probed` written with `anyprobe::fire!`.

#![allow(missing_docs)]

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

anyprobe::probes! {
    provider = "bench";
    fn work__entry(id: u64, label: &str);
    fn work__return(id: u64, result: u64);
}

#[inline(always)]
fn compute(id: u64, label: &str) -> u64 {
    id.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ label.len() as u64
}

#[cold]
#[inline(never)]
fn fire_entry(id: u64, label: &str) {
    work__entry::fire(id, label);
}

#[cold]
#[inline(never)]
fn fire_return(id: u64, result: u64) {
    work__return::fire(id, result);
}

#[inline(never)]
fn probed(id: u64, label: &str) -> u64 {
    if work__entry::enabled() {
        fire_entry(id, label);
    }
    let result = compute(id, label);
    if work__return::enabled() {
        fire_return(id, result);
    }
    result
}

#[inline(never)]
fn fire_macro(id: u64, label: &str) -> u64 {
    anyprobe::fire!(work__entry(id, label));
    let result = compute(id, label);
    anyprobe::fire!(work__return(id, result));
    result
}

#[anyprobe::probe(provider = "bench", ret = native)]
#[inline(never)]
fn attribute(id: u64, label: &str) -> u64 {
    compute(id, label)
}

#[inline(never)]
fn baseline(id: u64, label: &str) -> u64 {
    compute(id, label)
}

fn disabled_cost(c: &mut Criterion) {
    let mut group = c.benchmark_group("disabled_cost");
    group.bench_function("baseline", |b| {
        b.iter(|| baseline(black_box(42), black_box("label")));
    });
    group.bench_function("probed", |b| {
        b.iter(|| probed(black_box(42), black_box("label")));
    });
    group.bench_function("fire_macro", |b| {
        b.iter(|| fire_macro(black_box(42), black_box("label")));
    });
    group.bench_function("attribute", |b| {
        b.iter(|| attribute(black_box(42), black_box("label")));
    });
    group.finish();
}

criterion_group!(benches, disabled_cost);
criterion_main!(benches);
