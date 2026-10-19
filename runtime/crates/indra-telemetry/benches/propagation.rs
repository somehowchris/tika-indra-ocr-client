use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

fn propagation(criterion: &mut Criterion) {
    criterion.bench_function("w3c_parent_fixed_array_parse", |bench| {
        bench.iter(|| {
            black_box(indra_telemetry::Parent::parse(black_box(
                "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            )))
        });
    });
}

criterion_group!(benches, propagation);
criterion_main!(benches);
