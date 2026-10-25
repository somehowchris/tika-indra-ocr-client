//! Scheduler construction, sorting and selection cost for two pages. Both arms
//! start from the same unsorted widths on every iteration; these timings cannot
//! predict native OCR speedup or account for the pilot's extra live page memory.
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use indra_engine::{EngineError, MAX_BATCH, MAX_LINES, Result};
use std::{hint::black_box, time::Duration};

#[path = "../src/batching.rs"]
#[allow(dead_code, unused_imports)]
mod batching;
#[path = "../src/continuous/plan.rs"]
#[allow(dead_code, unused_imports)]
mod plan;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn selection(criterion: &mut Criterion) {
    let widths: [[usize; MAX_LINES]; 2] = std::array::from_fn(|owner| {
        std::array::from_fn(|index| 8 + (index * 73 + owner * 19) % 4089)
    });
    let flags = [false; MAX_LINES];
    let mut group = criterion.benchmark_group("continuous_selection");
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    group.sample_size(100);
    for count in [16, 64, 256, 1024] {
        for cap in [1, 8, 64] {
            group.bench_function(
                BenchmarkId::new(format!("serial-b{cap}"), count),
                |bencher| {
                    bencher.iter(|| {
                        for owner in &widths {
                            let active = black_box(&owner[..count]);
                            let mut queue =
                                batching::Plan::new(active, cap, batching::Policy::Text, |_| true)
                                    .expect("serial plan");
                            while let Some(range) = queue.next(active) {
                                black_box(&queue.indices[range]);
                            }
                        }
                    });
                },
            );
            group.bench_function(
                BenchmarkId::new(format!("continuous-b{cap}"), count),
                |bencher| {
                    let mut batch = plan::Batch::new();
                    bencher.iter(|| {
                        let widths = black_box(&widths);
                        let mut indices: [[usize; MAX_LINES]; 2] =
                            std::array::from_fn(|_| std::array::from_fn(|i| i));
                        for (owner, sorted) in indices.iter_mut().enumerate() {
                            sorted[..count].sort_unstable_by_key(|i| (widths[owner][*i], *i));
                        }
                        let mut cursor = [0; 2];
                        let mut anchor = 0;
                        while cursor.iter().any(|cursor| *cursor < count) {
                            if cursor[anchor] == count {
                                anchor = 1 - anchor;
                            }
                            let views = std::array::from_fn(|owner| {
                                (cursor[owner] < count).then_some(plan::View {
                                    widths: &widths[owner][..count],
                                    indices: &indices[owner][..count],
                                    flags: &flags[..count],
                                    route: None,
                                    position: cursor[owner],
                                })
                            });
                            plan::select(black_box(views), anchor, cap, &mut batch)
                                .expect("continuous plan");
                            for (owner, cursor) in cursor.iter_mut().enumerate() {
                                if batch.owners[owner] != 0 {
                                    *cursor = batch.end[owner];
                                }
                            }
                            black_box(&batch.rows[..batch.count]);
                            anchor = 1 - anchor;
                        }
                    });
                },
            );
        }
    }
    group.finish();
}

criterion_group!(benches, selection);
criterion_main!(benches);
