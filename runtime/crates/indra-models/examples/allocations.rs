use std::{alloc::System, path::Path};

use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static ALLOCATOR: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn main() {
    let directory = std::env::var_os("INDRA_TEST_MODEL_DIR").expect("INDRA_TEST_MODEL_DIR");
    let path = Path::new(&directory);
    indra_models::verify(path, true).expect("warm SHA dispatch and filesystem cache");
    for (profile, math) in [("text", false), ("math", true)] {
        let region = Region::new(ALLOCATOR);
        for _ in 0..100 {
            std::hint::black_box(indra_models::verify(path, math).expect("verified model set"));
        }
        let stats = region.change();
        // Formatting happens after observation. Counts describe System, not mimalloc.
        println!(
            "{{\"profile\":\"{profile}\",\"iterations\":100,\"allocations\":{},\"reallocations\":{},\"deallocations\":{},\"bytes_allocated\":{},\"bytes_deallocated\":{},\"bytes_reallocated\":{}}}",
            stats.allocations,
            stats.reallocations,
            stats.deallocations,
            stats.bytes_allocated,
            stats.bytes_deallocated,
            stats.bytes_reallocated
        );
    }
}
