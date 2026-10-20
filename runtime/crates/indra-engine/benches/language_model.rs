//! Pure-Rust costs of the 1.1 additions: n-gram lookups, the beam search and the bidi flip.
use std::{hint::black_box, time::Duration};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use indra_engine::gates::{BeamScratch, BidiScratch, LmParams, MAX_TOKENS, beam_search, flip};

#[path = "../tests/support/language_model.rs"]
mod language_model;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn lm_logp(criterion: &mut Criterion) {
    let model = language_model::synthetic_model(&ALLOCATOR);
    let mut group = criterion.benchmark_group("lm_logp");
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    let keys: Vec<u64> = (0..1024_u64)
        .map(|i| ((i * 37 % 1700) << 22) | ((i * 11 % 1700 + 1) << 11) | (i * 7 % 1700 + 1))
        .collect();
    group.bench_function("1024-keys", |bencher| {
        bencher.iter(|| {
            let mut total = 0.0_f64;
            for key in &keys {
                total += model.logp(black_box(*key));
            }
            black_box(total)
        });
    });
    group.finish();
}

fn beam(criterion: &mut Criterion) {
    let model = language_model::synthetic_model(&ALLOCATOR);
    let mut scratch = BeamScratch::new();
    let mut ids = [0_i32; MAX_TOKENS];
    let mut steps = [0_u16; MAX_TOKENS];
    let params = LmParams {
        alpha: 0.5,
        beta: 1.5,
        gate: 0.9,
    };
    let mut group = criterion.benchmark_group("beam_search");
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    for timesteps in [32, 96, 256, 512] {
        let line = language_model::synthetic_line(timesteps);
        group.bench_with_input(
            BenchmarkId::new("steps", timesteps),
            &timesteps,
            |bencher, _| {
                bencher.iter(|| {
                    black_box(
                        beam_search(
                            black_box(&line.blank),
                            black_box(&line.topv),
                            black_box(&line.topi),
                            &model,
                            params,
                            &mut scratch,
                            &mut ids,
                            &mut steps,
                        )
                        .expect("bounded search"),
                    )
                });
            },
        );
    }
    group.finish();
}

fn rtl_flip(criterion: &mut Criterion) {
    let mut scratch = Box::new(BidiScratch::new());
    let mut out = String::with_capacity(8192);
    let mut group = criterion.benchmark_group("rtl_flip");
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    for (name, text, base) in [
        ("rtl-page", language_model::RTL_LINE, Some(true)),
        ("ltr-line", language_model::MIXED_LINE, None),
    ] {
        group.bench_function(name, |bencher| {
            bencher.iter(|| {
                out.clear();
                flip(black_box(text), base, &mut scratch, &mut out).expect("bounded flip");
                black_box(out.len())
            });
        });
    }
    group.finish();
}

criterion_group!(benches, lm_logp, beam, rtl_flip);
criterion_main!(benches);
