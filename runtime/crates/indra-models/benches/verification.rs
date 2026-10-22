use std::{hint::black_box, path::Path, time::Duration};

use criterion::{Criterion, criterion_group, criterion_main};

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn verification(criterion: &mut Criterion) {
    let directory = std::env::var_os("INDRA_TEST_MODEL_DIR").expect("INDRA_TEST_MODEL_DIR");
    let path = Path::new(&directory);
    indra_models::verify(path, true).expect("fully verified warm file cache");
    let mut group = criterion.benchmark_group("offline_model_verification");
    group.sample_size(100);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    for (label, math) in [("text_9_files", false), ("math_14_files", true)] {
        group.bench_function(label, |bencher| {
            bencher.iter(|| {
                black_box(indra_models::verify(black_box(path), black_box(math)).expect("verify"))
            });
        });
    }
    group.finish();
}

criterion_group!(benches, verification);
criterion_main!(benches);
