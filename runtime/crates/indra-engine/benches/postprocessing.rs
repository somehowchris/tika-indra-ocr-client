//! Measure bounded CPU control work separately from ONNX/OpenCV kernel time.
use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use indra_engine::{EngineError, MAX_BATCH, MAX_LINES, Result};

#[path = "../src/batching.rs"]
#[allow(dead_code, unused_imports)]
mod batching;

fn width_planning(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("recognition_width_plan");
    let mut widths = [0_usize; MAX_LINES];
    for (index, width) in widths.iter_mut().enumerate() {
        *width = 8 + (index * 73) % 4089;
    }
    for count in [16, 64, 256, 1024] {
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &count,
            |bencher, count| {
                bencher.iter(|| {
                    let active = black_box(&widths[..*count]);
                    let mut plan =
                        batching::Plan::new(active, 64, batching::Policy::Text, |_| true)
                            .expect("fixed valid widths");
                    let mut batches = 0;
                    while let Some(range) = plan.next(active) {
                        black_box(&plan.indices[range]);
                        batches += 1;
                    }
                    black_box(batches)
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, width_planning);
criterion_main!(benches);
