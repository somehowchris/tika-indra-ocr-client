//! Model-backed observation of Rust allocator activity on the warmed request
//! path. A standalone executable keeps libtest's own allocations out of the
//! counters. Every repeated request must show identical six-counter activity
//! (no growth, no leak) and the per-request churn is reported for attribution.
//! Native allocations inside ONNX Runtime and OpenCV are not Rust allocator
//! calls and stay outside these numbers.
use std::time::{Duration, Instant};

use indra_engine::{Engine, MAX_OUTPUT_BYTES};
use stats_alloc::{Region, Stats, StatsAlloc};

mod support;

#[global_allocator]
static ALLOCATOR: StatsAlloc<mimalloc::MiMalloc> = StatsAlloc::new(mimalloc::MiMalloc);

static FIXTURES: [(&str, &[u8]); 5] = [
    (
        "smoke",
        include_bytes!("../../../../backend/tests/data/smoke.png"),
    ),
    (
        "two-pages",
        include_bytes!("../../../../backend/tests/data/two-pages.tiff"),
    ),
    (
        "synthetic-rtl",
        include_bytes!("evidence/2026-10-21-parity-1-1/fixtures/synthetic-rtl.png"),
    ),
    (
        "synthetic-world",
        include_bytes!("evidence/2026-10-21-parity-1-1/fixtures/synthetic-world.png"),
    ),
    (
        "synthetic-vertical-cjk",
        include_bytes!("evidence/2026-10-21-parity-1-1/fixtures/synthetic-vertical-cjk.png"),
    ),
];
const REPEATS: usize = 5;

fn main() {
    let directory = support::directory();
    let timeout = Duration::from_secs(120);
    for math in [false, true] {
        let settings = support::settings(math);
        let mut engine = Engine::load(&directory, &settings).expect("pinned engine");
        engine
            .warmup(Instant::now() + timeout)
            .expect("warmed sessions");
        let mut output = String::with_capacity(MAX_OUTPUT_BYTES);
        for (name, bytes) in FIXTURES {
            engine
                .recognize(bytes, Instant::now() + timeout, &mut output)
                .expect("warm request");
            let mut observed = [Stats::default(); REPEATS];
            for slot in &mut observed {
                let region = Region::new(&ALLOCATOR);
                engine
                    .recognize(bytes, Instant::now() + timeout, &mut output)
                    .expect("repeated request");
                *slot = region.change();
            }
            let stats = observed[0];
            println!(
                "{{\"fixture\":\"{name}\",\"math\":{math},\"threads\":{},\"repeats\":{REPEATS},\
                 \"allocations\":{},\"deallocations\":{},\"reallocations\":{},\
                 \"bytes_allocated\":{},\"bytes_deallocated\":{},\"bytes_reallocated\":{}}}",
                settings.threads,
                stats.allocations,
                stats.deallocations,
                stats.reallocations,
                stats.bytes_allocated,
                stats.bytes_deallocated,
                stats.bytes_reallocated
            );
            assert!(
                observed.iter().all(|repeat| *repeat == stats),
                "request-path allocator activity must be identical across warmed requests: {observed:?}"
            );
        }
    }
}
