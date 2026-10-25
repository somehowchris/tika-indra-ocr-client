//! Rust allocator activity of warmed proxied requests through the real
//! gateway (`service::serve`) in this process. The loopback backend and the
//! client are threads that do not allocate, and a parked child process stands
//! in for Java, so the counters are the gateway's own: admission, Axum and
//! Hyper on both sides, the exchange task and response streaming. Telemetry
//! is not initialized; tracing events are disabled no-ops here, and JSON log
//! formatting and exporters stay outside these numbers.
//!
//! A standalone executable keeps libtest's own allocations out of the
//! process-wide counters. Every measured request must show identical six
//! counters, free what it allocated, and stay within the committed ceiling;
//! any increase fails this gate. Lower the ceiling with a change that removes
//! allocations.
use indra_gateway::{
    commands,
    configuration::{Gateway, ProbeKind, Settings},
    service,
};
use stats_alloc::{Region, Stats, StatsAlloc};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    thread,
    time::{Duration, Instant},
};

#[global_allocator]
static ALLOCATOR: StatsAlloc<mimalloc::MiMalloc> = StatsAlloc::new(mimalloc::MiMalloc);

/// Per warmed request, exact for each profile and target architecture.
/// Optimization changes a few library allocations. One small hash table per
/// request carries one SIMD control group, 16 bytes with x86_64 SSE2 and 8 on
/// aarch64, so x86_64 allocates 8 bytes more. rust-runtime.yml runs the
/// x86_64 debug ceiling.
static CEILING: Stats = {
    let (allocations, bytes) = match (cfg!(target_arch = "x86_64"), cfg!(debug_assertions)) {
        (true, true) => (46, 63_034),
        (true, false) => (45, 63_090),
        (false, true) => (46, 63_026),
        (false, false) => (45, 63_082),
    };
    Stats {
        allocations,
        deallocations: allocations,
        reallocations: 3,
        bytes_allocated: bytes,
        bytes_deallocated: bytes,
        bytes_reallocated: 9_128,
    }
};
static WARMUP: usize = 3;
const REPEATS: usize = 5;
static REQUEST: &[u8] =
    b"PUT /tika HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello";
static RESPONSE: &[u8] =
    b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 6\r\nConnection: close\r\n\r\nparsed";
static VERSION: &[u8] =
    b"HTTP/1.1 200 OK\r\nContent-Length: 17\r\nConnection: close\r\n\r\nApache Tika 3.3.2";

/// Supervisor version probes accepted and completed by the backend.
static PROBES_STARTED: AtomicUsize = AtomicUsize::new(0);
static PROBES_FINISHED: AtomicUsize = AtomicUsize::new(0);
/// Parses whose upstream connection the gateway has closed.
static PARSES_FINISHED: AtomicUsize = AtomicUsize::new(0);

fn main() {
    if std::env::args().any(|argument| argument == "-jar") {
        // The supervised "Java" only has to live until its group is signalled.
        loop {
            thread::park();
        }
    }
    let backend = TcpListener::bind("127.0.0.1:0").unwrap();
    let settings = Settings {
        gateway: Gateway {
            listen: TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap(),
            backend: backend.local_addr().unwrap(),
            java: std::env::current_exe().unwrap(),
            jar: PathBuf::from("/fixture.jar"),
            tika_config: PathBuf::from("/fixture.xml"),
            max_active: 1,
            max_connections: 8,
            max_request_bytes: 1024,
            max_response_bytes: 1024,
            ..Gateway::default()
        },
        ..Settings::default()
    };
    let public = settings.gateway.listen;
    thread::spawn(move || serve_backend(&backend));
    let gateway = thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(service::serve(settings))
    });
    wait_for(|| commands::probe(public, ProbeKind::Ready).is_ok());
    for _ in 0..WARMUP {
        parse(public);
    }
    let mut observed = [Stats::default(); REPEATS];
    for slot in &mut observed {
        *slot = measured(public);
    }
    commands::drain(public).unwrap();
    gateway.join().unwrap().unwrap();
    let stats = observed[0];
    println!(
        "{{\"scope\":\"gateway proxied PUT /tika, warmed\",\"allocator_counter\":\"MiMalloc\",\"repeats\":{REPEATS},\
         \"allocations\":{},\"deallocations\":{},\"reallocations\":{},\
         \"bytes_allocated\":{},\"bytes_deallocated\":{},\"bytes_reallocated\":{}}}",
        stats.allocations,
        stats.deallocations,
        stats.reallocations,
        stats.bytes_allocated,
        stats.bytes_deallocated,
        stats.bytes_reallocated
    );
    assert!(
        observed.iter().all(|repeat| *repeat == stats),
        "proxy allocator activity must be identical across warmed requests: {observed:?}"
    );
    assert_eq!(
        (stats.allocations, stats.bytes_allocated),
        (stats.deallocations, stats.bytes_deallocated),
        "a finished request frees everything it allocated"
    );
    assert!(
        stats.allocations <= CEILING.allocations
            && stats.deallocations <= CEILING.deallocations
            && stats.reallocations <= CEILING.reallocations
            && stats.bytes_allocated <= CEILING.bytes_allocated
            && stats.bytes_deallocated <= CEILING.bytes_deallocated
            && stats.bytes_reallocated <= CEILING.bytes_reallocated,
        "proxy allocations exceed the committed ceiling {CEILING:?}: {stats:?}"
    );
}

/// One parse counted from the client's connect until the gateway has
/// released its upstream connection, exchange task and frontend. A
/// supervisor probe that overlaps the region would add its own allocations,
/// so that sample is taken again; probes start about once per second.
fn measured(public: SocketAddr) -> Stats {
    loop {
        let finished = PROBES_FINISHED.load(Ordering::Acquire);
        wait_for(|| PROBES_FINISHED.load(Ordering::Acquire) > finished);
        // The supervisor publishes the probe result after the reply.
        thread::sleep(Duration::from_millis(20));
        let started = PROBES_STARTED.load(Ordering::Acquire);
        let region = Region::new(&ALLOCATOR);
        parse(public);
        // The exchange task and connection drop right after the upstream
        // close that parse() waits for; give the gateway thread that moment.
        thread::sleep(Duration::from_millis(20));
        let stats = region.change();
        // A probe connection opened inside the region is accepted promptly.
        thread::sleep(Duration::from_millis(20));
        if PROBES_STARTED.load(Ordering::Acquire) == started {
            return stats;
        }
    }
}

fn parse(public: SocketAddr) {
    let parses = PARSES_FINISHED.load(Ordering::Acquire);
    let mut socket = TcpStream::connect(public).unwrap();
    socket.write_all(REQUEST).unwrap();
    let mut response = [0; 4096];
    let mut used = 0;
    loop {
        let read = socket.read(&mut response[used..]).unwrap();
        if read == 0 {
            break;
        }
        used += read;
    }
    assert!(response[..used].starts_with(b"HTTP/1.1 200 OK\r\n"));
    assert!(response[..used].ends_with(b"parsed"));
    wait_for(|| PARSES_FINISHED.load(Ordering::Acquire) > parses);
}

/// An allocation-free stand-in for Tika: fixed buffers, fixed answers.
fn serve_backend(listener: &TcpListener) {
    for stream in listener.incoming() {
        let mut socket = stream.unwrap();
        let mut head = [0; 2048];
        let mut used = 0;
        while !head[..used].windows(4).any(|end| end == b"\r\n\r\n") {
            let read = socket.read(&mut head[used..]).unwrap();
            assert!(read > 0, "complete upstream request head");
            used += read;
        }
        let version = head.starts_with(b"GET /version ");
        let (answer, finished) = if version {
            PROBES_STARTED.fetch_add(1, Ordering::AcqRel);
            (VERSION, &PROBES_FINISHED)
        } else {
            assert!(head.starts_with(b"PUT /tika "));
            (RESPONSE, &PARSES_FINISHED)
        };
        socket.write_all(answer).unwrap();
        // Hyper closes its side once the exchange is complete.
        let mut sink = [0; 64];
        while socket.read(&mut sink).is_ok_and(|read| read > 0) {}
        finished.fetch_add(1, Ordering::AcqRel);
    }
}

fn wait_for(mut predicate: impl FnMut() -> bool) {
    let due = Instant::now() + Duration::from_secs(10);
    while !predicate() {
        assert!(Instant::now() < due, "timed out");
        thread::sleep(Duration::from_millis(1));
    }
}
