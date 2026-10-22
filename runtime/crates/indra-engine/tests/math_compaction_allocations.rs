//! Standalone allocator evidence for the gather core; excludes ORT and diagnostics.
use stats_alloc::{Region, StatsAlloc};
use std::{alloc::System, hint::black_box};
#[global_allocator]
static ALLOCATOR: StatsAlloc<System> = StatsAlloc::system();
#[path = "../src/math/gather.rs"]
mod gather;
fn main() {
    let mut cache = [0_f32; 3 * 6 * 8 * 32 * 24];
    for (index, value) in cache.iter_mut().enumerate() {
        *value = index as f32;
    }
    let region = Region::new(&ALLOCATOR);
    let mut bytes = 0;
    for _ in 0..1000 {
        bytes +=
            gather::gather::<_, false>(black_box(&mut cache), 3, 6, &[1, 3, 4, 5], 8 * 32 * 24)
                .expect("bounded gather")
                .bytes;
        bytes += gather::gather::<_, false>(black_box(&mut cache), 3, 4, &[0, 1, 2], 8 * 32 * 24)
            .expect("bounded gather")
            .bytes;
        bytes += gather::gather::<_, false>(black_box(&mut cache), 3, 3, &[0, 2], 8 * 32 * 24)
            .expect("bounded gather")
            .bytes;
        bytes += gather::gather::<_, false>(black_box(&mut cache), 3, 2, &[1], 8 * 32 * 24)
            .expect("bounded gather")
            .bytes;
    }
    black_box(bytes);
    let stats = region.change();
    assert_eq!(stats.allocations, 0);
    assert_eq!(stats.reallocations, 0);
    assert_eq!(stats.deallocations, 0);
    assert_eq!(stats.bytes_allocated, 0);
    assert_eq!(stats.bytes_deallocated, 0);
    assert_eq!(stats.bytes_reallocated, 0);
    println!(
        "{{\"scope\":\"safe in-place gather core only; prepared reused arena\",\"gathers\":4000,\"copied_bytes\":{bytes},\"allocations\":0,\"reallocations\":0,\"deallocations\":0,\"bytes_allocated\":0,\"bytes_deallocated\":0,\"bytes_reallocated\":0}}"
    );
}
