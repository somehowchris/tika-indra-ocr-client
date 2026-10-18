use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::{alloc::System, hint::black_box};

#[global_allocator]
static ALLOCATOR: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn main() {
    let region = Region::new(ALLOCATOR);
    for _ in 0..10_000 {
        black_box(indra_telemetry::Parent::parse(black_box(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        )));
    }
    let counts = region.change();
    assert_eq!(counts.allocations, 0);
    assert_eq!(counts.reallocations, 0);
    assert_eq!(counts.deallocations, 0);
    println!(
        "{{\"operation\":\"strict_traceparent_parse\",\"iterations\":10000,\"allocator\":\"System\",\"allocations\":{},\"reallocations\":{},\"deallocations\":{}}}",
        counts.allocations, counts.reallocations, counts.deallocations
    );
}
