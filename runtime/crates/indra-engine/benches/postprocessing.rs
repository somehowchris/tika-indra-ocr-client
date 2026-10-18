//! Measure bounded CPU control work separately from ONNX/OpenCV kernel time.
use std::{hint::black_box, time::Duration};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use indra_engine::{EngineError, MAX_BATCH, MAX_LINES, Result};

#[path = "../src/batching.rs"]
#[allow(dead_code, unused_imports)]
mod batching;

#[path = "../tests/support/width_scan.rs"]
mod width_scan;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn width_planning(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("recognition_width_plan");
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    group.sample_size(100);
    let mut widths = [0_usize; MAX_LINES];
    for (index, width) in widths.iter_mut().enumerate() {
        *width = 8 + (index * 73) % 4089;
    }
    for count in [16, 64, 256, 1024] {
        for cap in [1, 8, 64] {
            for math in [false, true] {
                let name = if math { "math" } else { "text" };
                group.bench_with_input(
                    BenchmarkId::new(format!("{name}-b{cap}"), count),
                    &count,
                    |bencher, count| {
                        bencher.iter(|| {
                            let active = black_box(&widths[..*count]);
                            let policy = if math {
                                batching::Policy::Math
                            } else {
                                batching::Policy::Text
                            };
                            let mut plan = batching::Plan::new(active, cap, policy, |_| true)
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
        }
    }
    group.finish();
}

fn tensor_width_scan(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("recognition_tensor_width_scan");
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    group.sample_size(100);
    let mut widths = [0; MAX_BATCH];
    for case in &width_scan::CASES {
        case.fill(&mut widths);
        let active = &widths[..case.count];
        let mut metadata = [0; MAX_BATCH];
        group.bench_function(BenchmarkId::new("legacy_max", case.name), |bencher| {
            bencher.iter(|| {
                black_box(width_scan::legacy_max(
                    black_box(active),
                    black_box(case.height),
                    black_box(&mut metadata),
                ))
            });
        });
        group.bench_function(BenchmarkId::new("proved_plan", case.name), |bencher| {
            bencher.iter(|| {
                black_box(
                    batching::tensor_widths(
                        black_box(active).iter().copied().map(Ok),
                        black_box(case.height),
                        black_box(case.height * case.columns),
                        black_box(&mut metadata),
                    )
                    .expect("prespecified shape fits the production arena"),
                )
            });
        });
    }
    group.finish();
}

criterion_group!(benches, width_planning, tensor_width_scan);
criterion_main!(benches);
