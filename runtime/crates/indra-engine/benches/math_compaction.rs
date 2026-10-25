//! Gather cost only; no native decoder speed claim.
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
#[path = "../src/math/gather.rs"]
mod gather;
fn core(c: &mut Criterion) {
    let mut arena = [0_f32; 3 * 6 * 8 * 32 * 24];
    for (index, value) in arena.iter_mut().enumerate() {
        *value = index as f32;
    }
    let cases: [(usize, &[usize], usize); 4] = [
        (6, &[1, 3, 4, 5], 8),
        (4, &[0, 1, 2], 23),
        (3, &[0, 2], 27),
        (2, &[1], 32),
    ];
    for (old, rows, steps) in cases {
        c.bench_function(
            &format!("math/self-cache-gather/{old}-to-{}-at-{steps}", rows.len()),
            |b| {
                b.iter(|| {
                    black_box(
                        gather::gather::<_, false>(
                            black_box(&mut arena),
                            3,
                            old,
                            black_box(rows),
                            8 * steps * 24,
                        )
                        .expect("bounded gather"),
                    )
                });
            },
        );
    }
}
criterion_group!(benches, core);
criterion_main!(benches);
