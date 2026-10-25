//! Measure bounded CPU control work separately from ONNX/OpenCV kernel time.
use std::{hint::black_box, time::Duration};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use indra_engine::{EngineError, MAX_BATCH, MAX_LINES, Result};

#[path = "../src/batching.rs"]
#[allow(dead_code, unused_imports)]
mod batching;

#[path = "../tests/support/width_scan.rs"]
mod width_scan;

#[path = "../src/geometry/projection.rs"]
#[allow(dead_code, unused_imports)]
mod projection;

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

/// The row sum `projection::sum` replaced, retained only as a comparison arm:
/// under the release profile's overflow checks one checked add per byte.
fn legacy_sharpness(pixels: &[u8], width: usize) -> f64 {
    let mut total = 0.0_f64;
    let mut prior = 0_i64;
    for (index, row) in pixels.chunks_exact(width).enumerate() {
        let value = row.iter().map(|pixel| i64::from(*pixel)).sum::<i64>();
        if index != 0 {
            let difference = (value - prior) as f64;
            total += difference * difference;
        }
        prior = value;
    }
    total
}

fn skew_row_sharpness(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("skew_row_sharpness");
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    group.sample_size(100);
    let mut pixels = [0_u8; 564 * 800];
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    for pixel in &mut pixels {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *pixel = (state >> 24) as u8;
    }
    // Width × height of the fine and coarse skew thumbnails of the math
    // (preview 1128 × 1600) and TIFF (1100 × 220) profile fixtures.
    for (name, width, height) in [
        ("564x800", 564, 800),
        ("282x400", 282, 400),
        ("800x160", 800, 160),
        ("400x80", 400, 80),
    ] {
        let thumbnail = &pixels[..width * height];
        assert_eq!(
            projection::sharpness(thumbnail, width).to_bits(),
            legacy_sharpness(thumbnail, width).to_bits()
        );
        group.bench_function(BenchmarkId::new("legacy_checked_i64", name), |bencher| {
            bencher.iter(|| black_box(legacy_sharpness(black_box(thumbnail), black_box(width))));
        });
        group.bench_function(BenchmarkId::new("block_u32", name), |bencher| {
            bencher.iter(|| {
                black_box(projection::sharpness(
                    black_box(thumbnail),
                    black_box(width),
                ))
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    width_planning,
    tensor_width_scan,
    skew_row_sharpness
);
criterion_main!(benches);
