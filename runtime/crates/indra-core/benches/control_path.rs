use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use indra_core::{batch::BatchPlan, lifecycle::Lifecycle, queue::Queue};

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn control_path(criterion: &mut Criterion) {
    let lifecycle = Lifecycle::new();
    lifecycle.activate().expect("benchmark starts ready");
    criterion.bench_function("admission/release", |bencher| {
        bencher.iter(|| drop(black_box(lifecycle.admit(black_box(4)).expect("available"))));
    });
    let mut queue = Queue::<u64, 8>::new();
    criterion.bench_function("queue/push_pop", |bencher| {
        bencher.iter(|| {
            queue.push(black_box(42)).expect("bounded queue is empty");
            black_box(queue.pop());
        });
    });
    let mut batch = BatchPlan::new(64, 40960);
    criterion.bench_function("batch/64_lines", |bencher| {
        bencher.iter(|| {
            batch.clear();
            for width in 1..=64 {
                batch
                    .try_add(black_box(width * 8))
                    .expect("within tensor budget");
            }
            black_box(batch.work());
        });
    });
}

criterion_group!(benches, control_path);
criterion_main!(benches);
