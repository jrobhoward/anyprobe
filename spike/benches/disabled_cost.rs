//! Cost of a probed function with no tracer attached, against the same
//! function with no probes.

#![allow(missing_docs)]

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

fn disabled_cost(c: &mut Criterion) {
    let mut group = c.benchmark_group("disabled_cost");
    group.bench_function("baseline", |b| {
        b.iter(|| anyprobe_spike::baseline(black_box(42), black_box("label")));
    });
    group.bench_function("probed", |b| {
        b.iter(|| anyprobe_spike::work_outlined(black_box(42), black_box("label")));
    });
    group.finish();
}

criterion_group!(benches, disabled_cost);
criterion_main!(benches);
