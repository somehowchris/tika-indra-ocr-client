//! Instrument Rust allocator calls independently from Criterion's own reporting
//! allocations. This does not measure allocations inside native inference.

use std::{alloc::System, hint::black_box};

use indra_core::{batch::BatchPlan, lifecycle::Lifecycle, queue::Queue};
use stats_alloc::{Region, StatsAlloc};

#[global_allocator]
static ALLOCATOR: StatsAlloc<System> = StatsAlloc::system();

#[test]
fn accepted_control_operations_do_not_allocate_or_reallocate() {
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
}
