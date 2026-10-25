//! Model-backed gate on Rust allocator activity on the warmed request path. A
//! standalone executable keeps libtest's own allocations out of the counters.
//! Every repeated request must show identical six-counter activity (no growth,
//! no leak), free what it allocates, and stay within the committed per-fixture
//! ceiling. Native allocations inside ONNX Runtime and OpenCV are not Rust
//! allocator calls and stay outside these numbers.
use std::time::{Duration, Instant};

use indra_engine::{Engine, MAX_OUTPUT_BYTES};
use stats_alloc::{Region, Stats, StatsAlloc};

mod support;

#[global_allocator]
static ALLOCATOR: StatsAlloc<mimalloc::MiMalloc> = StatsAlloc::new(mimalloc::MiMalloc);

/// Six counters of a request that frees everything it allocates and never
/// reallocates.
const fn balanced(allocations: usize, bytes: usize) -> Stats {
    Stats {
        allocations,
        deallocations: allocations,
        reallocations: 0,
        bytes_allocated: bytes,
        bytes_deallocated: bytes,
        bytes_reallocated: 0,
    }
}

/// Each fixture with its per-request ceiling without and with math: the
/// counters native acceptance run 39839055887 measured at one and two CPUs
/// (the same in run 39766169169). They are ort 2.0.0-rc.14's per-value
/// wrappers (docs/native-engine.md); a change that adds a Rust allocation to
/// the request path fails here. Lower a ceiling when a change removes churn,
/// and re-measure it when ort or the models change.
static FIXTURES: [(&str, &[u8], [Stats; 2]); 5] = [
    (
        "smoke",
        include_bytes!("../../../../backend/tests/data/smoke.png"),
        [balanced(28, 4_040), balanced(32, 4_752)],
    ),
    (
        "two-pages",
        include_bytes!("../../../../backend/tests/data/two-pages.tiff"),
        [balanced(56, 8_080), balanced(64, 9_504)],
    ),
    (
        "synthetic-rtl",
        include_bytes!("evidence/2026-10-21-parity-1-1/fixtures/synthetic-rtl.png"),
        [balanced(60, 8_608), balanced(208, 27_632)],
    ),
    (
        "synthetic-world",
        include_bytes!("evidence/2026-10-21-parity-1-1/fixtures/synthetic-world.png"),
        [balanced(78, 10_960), balanced(86, 12_384)],
    ),
    (
        "synthetic-vertical-cjk",
        include_bytes!("evidence/2026-10-21-parity-1-1/fixtures/synthetic-vertical-cjk.png"),
        [balanced(52, 7_184), balanced(200, 26_208)],
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
        for (name, bytes, ceilings) in FIXTURES {
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
            assert_eq!(
                (stats.deallocations, stats.bytes_deallocated),
                (stats.allocations, stats.bytes_allocated),
                "{name} math={math}: a request must free what it allocates"
            );
            let ceiling = ceilings[usize::from(math)];
            assert!(
                stats.allocations <= ceiling.allocations
                    && stats.deallocations <= ceiling.deallocations
                    && stats.reallocations <= ceiling.reallocations
                    && stats.bytes_allocated <= ceiling.bytes_allocated
                    && stats.bytes_deallocated <= ceiling.bytes_deallocated
                    && stats.bytes_reallocated <= ceiling.bytes_reallocated,
                "{name} math={math}: {stats:?} exceeds the committed ceiling {ceiling:?}"
            );
        }
    }
}
