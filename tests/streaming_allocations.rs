use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::{
    alloc::System,
    io::{self, BufReader, Cursor},
};
use tika_indra_ocr_client::{MAX_OUTPUT, headers, stream_tika_text};

#[global_allocator]
static ALLOCATOR: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

static BODY: [u8; MAX_OUTPUT] = {
    let mut data = [b'a'; MAX_OUTPUT];
    // A four-byte codepoint straddles the actual 8-KiB reader boundary.
    data[8191] = 0xf0;
    data[8192] = 0x9f;
    data[8193] = 0xa7;
    data[8194] = 0xaa;
    data[0] = 0x0c;
    data[8190] = 0x0c;
    data[MAX_OUTPUT - 1] = 0x0c;
    data
};
static HEADERS: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 4194304\r\n\r\n";

// This target has harness=false: process-wide counts must not include libtest's
// concurrent reporting allocations.
fn main() {
    // BufReader owns one transport buffer per bridge process. Construct it
    // before the region so this isolates the changed header/body algorithm.
    let mut reader = BufReader::with_capacity(8192, Cursor::new(&BODY[..]));
    let region = Region::new(ALLOCATOR);
    let response = headers(&mut Cursor::new(HEADERS)).expect("bounded headers");
    stream_tika_text(&mut reader, response.length, &mut io::sink())
        .expect("maximum valid UTF-8 response");
    let change = region.change();
    assert_eq!(change.allocations, 0);
    assert_eq!(change.reallocations, 0);
    assert_eq!(change.deallocations, 0);
    assert_eq!(change.bytes_allocated, 0);
    assert_eq!(change.bytes_deallocated, 0);
    assert_eq!(change.bytes_reallocated, 0);
    println!(
        "{{\"scope\":\"bridge fixed headers and 4 MiB UTF-8 streaming with empty page separator\",\"allocator_counter\":\"System\",\"allocations\":0,\"reallocations\":0,\"deallocations\":0,\"bytes_allocated\":0,\"bytes_reallocated\":0,\"bytes_deallocated\":0}}"
    );
}
