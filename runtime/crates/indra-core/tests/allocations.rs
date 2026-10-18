//! Instrument Rust allocator calls independently from Criterion's own reporting
//! allocations. A standalone executable also excludes libtest's concurrent
//! reporting from the process-wide allocator counter. This does not measure
//! allocations inside native inference.

use std::{alloc::System, hint::black_box};

use indra_core::{batch::BatchPlan, lifecycle::Lifecycle, queue::Queue};
use stats_alloc::{Region, StatsAlloc};

#[global_allocator]
static ALLOCATOR: StatsAlloc<System> = StatsAlloc::system();

fn main() {
    let lifecycle = Lifecycle::new();
    lifecycle.activate().expect("starting");
    let mut queue = Queue::<u64, 8>::new();
    let mut batch = BatchPlan::new(64, 40960);
    let region = Region::new(&ALLOCATOR);
    for _ in 0..100_000 {
        let lease = lifecycle.admit(black_box(4)).expect("available capacity");
        queue.push(black_box(42)).expect("empty queue");
        black_box(queue.pop());
        batch.clear();
        batch.try_add(black_box(640)).expect("bounded crop");
        black_box(batch.work());
        drop(lease);
    }
    let observed = region.change();
    assert_eq!(observed.allocations, 0);
    assert_eq!(observed.reallocations, 0);
    assert_eq!(observed.deallocations, 0);
    assert_eq!(observed.bytes_allocated, 0);
    assert_eq!(observed.bytes_deallocated, 0);
    assert_eq!(observed.bytes_reallocated, 0);
    println!(
        "{{\"scope\":\"Rust admission, queue and batch control\",\"allocator_counter\":\"System\",\"iterations\":100000,\"allocations\":0,\"reallocations\":0,\"deallocations\":0,\"bytes_allocated\":0,\"bytes_reallocated\":0,\"bytes_deallocated\":0}}"
    );
}
