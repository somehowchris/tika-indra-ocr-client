//! Rust allocator activity at the HTTP service level. The public
//! `service::serve` runs in-process on the verified model snapshot, so its
//! native worker is really warmed. A std::net client with fixed request bytes
//! and a fixed read buffer drives liveness, readiness and the admission-full
//! rejection over loopback. A standalone executable keeps libtest's own
//! allocations out of the counters.
//!
//! One request covers the server's accept, Hyper connection, Axum routing,
//! handler and response, until every byte it allocated has been freed again.
//! Its six counters must be identical across warmed repeats and must not
//! exceed the committed ceiling. No tracing subscriber is installed, so JSON
//! logging and OTLP export are outside these numbers; native allocations
//! inside ONNX Runtime and OpenCV are not Rust allocator calls.
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    time::{Duration, Instant},
};

use indra_server::{configuration::Settings, service};
use stats_alloc::{Region, Stats, StatsAlloc};

#[global_allocator]
static ALLOCATOR: StatsAlloc<mimalloc::MiMalloc> = StatsAlloc::new(mimalloc::MiMalloc);

static WARMUP: usize = 3;
const REPEATS: usize = 5;

struct Case {
    name: &'static str,
    request: &'static [u8],
    status: &'static [u8],
    body: &'static [u8],
    /// Observed with indra-engine replaced by a stub, which none of these
    /// requests calls: linux/amd64 at opt-level 3 without LTO gave these
    /// values, and aarch64 with the release profile's thin LTO the same counts
    /// with 8 fewer bytes. Lower it deliberately when a change removes churn;
    /// any increase fails this gate.
    ceiling: Stats,
}

static HEALTH: Case = Case {
    name: "healthz",
    request: b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    status: b"HTTP/1.1 200 ",
    body: b"\"phase\":\"ready\"",
    ceiling: Stats {
        allocations: 26,
        deallocations: 26,
        reallocations: 1,
        bytes_allocated: 28735,
        bytes_deallocated: 28735,
        bytes_reallocated: 8192,
    },
};

static READY: Case = Case {
    name: "readyz",
    request: b"GET /readyz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    status: b"HTTP/1.1 200 ",
    body: b"\"phase\":\"ready\"",
    ceiling: Stats {
        allocations: 26,
        deallocations: 26,
        reallocations: 1,
        bytes_allocated: 28735,
        bytes_deallocated: 28735,
        bytes_reallocated: 8192,
    },
};

/// Sent while every admission slot holds a stalled upload. The declared body
/// byte is never sent: the service rejects before reading the body.
static QUEUE_FULL: Case = Case {
    name: "queue_full",
    request: b"POST /ocr HTTP/1.1\r\nHost: localhost\r\nContent-Type: image/png\r\nContent-Length: 1\r\nConnection: close\r\n\r\n",
    status: b"HTTP/1.1 503 ",
    body: b"\"code\":\"queue_full\"",
    ceiling: Stats {
        allocations: 31,
        deallocations: 31,
        reallocations: 3,
        bytes_allocated: 31787,
        bytes_deallocated: 31787,
        bytes_reallocated: 8944,
    },
};

/// One of two declared body bytes: the upload holds its admission slot.
static STALLED_UPLOAD: &[u8] = b"POST /ocr HTTP/1.1\r\nHost: localhost\r\nContent-Type: image/png\r\nContent-Length: 2\r\nConnection: close\r\n\r\nx";
static DRAIN: &[u8] =
    b"POST /drain HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

fn main() {
    let threads = match std::env::var("INDRA_TEST_THREADS") {
        Ok(value) => value
            .parse::<usize>()
            .expect("INDRA_TEST_THREADS is 1 or 2"),
        Err(std::env::VarError::NotPresent) => 1,
        Err(_) => panic!("INDRA_TEST_THREADS must be valid UTF-8"),
    };
    assert!(
        matches!(threads, 1 | 2),
        "INDRA_TEST_THREADS must be 1 or 2"
    );
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve test address");
    let address = listener.local_addr().expect("test address");
    drop(listener);
    let mut settings = Settings::default();
    settings.server.listen = address;
    // One worker plus one queued request: two stalled uploads fill admission.
    settings.server.queue_capacity = 1;
    settings.server.upload_timeout_seconds = 300;
    settings.server.request_timeout_seconds = 300;
    settings.server.shutdown_timeout_seconds = 310;
    settings.server.startup_timeout_seconds = 300;
    settings.engine.threads = threads;
    settings.engine.cv_threads = Some(threads);
    settings.engine.math = true;
    settings.models.directory = PathBuf::from(
        std::env::var_os("INDRA_TEST_MODEL_DIR")
            .expect("point INDRA_TEST_MODEL_DIR at the verified pinned snapshot"),
    );
    let server = std::thread::Builder::new()
        .name(String::from("indra-http"))
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("service runtime")
                .block_on(service::serve(settings))
        })
        .expect("service thread");

    let mut response = [0; 2048];
    let due = Instant::now() + Duration::from_secs(300);
    while !exchange(address, READY.request, &mut response)
        .is_ok_and(|used| response[..used].starts_with(READY.status))
    {
        assert!(!server.is_finished(), "service must warm its worker");
        assert!(Instant::now() < due, "service readiness deadline");
        std::thread::sleep(Duration::from_millis(50));
    }
    observe(&HEALTH, address, threads, &mut response);
    observe(&READY, address, threads, &mut response);

    let held: [TcpStream; 2] = std::array::from_fn(|_| {
        let mut upload = connect(address).expect("held upload connection");
        upload.write_all(STALLED_UPLOAD).expect("held upload head");
        upload
    });
    until_active(address, b"\"active\":2", &mut response);
    observe(&QUEUE_FULL, address, threads, &mut response);
    drop(held);
    until_active(address, b"\"active\":0", &mut response);

    let used = exchange(address, DRAIN, &mut response).expect("loopback drain");
    assert!(response[..used].starts_with(b"HTTP/1.1 200 "));
    server
        .join()
        .expect("service thread")
        .expect("drained service exits cleanly");
}

fn observe(case: &Case, address: SocketAddr, threads: usize, response: &mut [u8; 2048]) {
    for _ in 0..WARMUP {
        request(case, address, response);
    }
    let mut observed = [Stats::default(); REPEATS];
    for slot in &mut observed {
        quiescent();
        let region = Region::new(&ALLOCATOR);
        request(case, address, response);
        *slot = released(&region);
    }
    let stats = observed[0];
    let ceiling = case.ceiling;
    println!(
        "{{\"case\":\"{}\",\"threads\":{threads},\"repeats\":{REPEATS},\
         \"allocations\":{},\"deallocations\":{},\"reallocations\":{},\
         \"bytes_allocated\":{},\"bytes_deallocated\":{},\"bytes_reallocated\":{}}}",
        case.name,
        stats.allocations,
        stats.deallocations,
        stats.reallocations,
        stats.bytes_allocated,
        stats.bytes_deallocated,
        stats.bytes_reallocated
    );
    assert!(
        observed.iter().all(|repeat| *repeat == stats),
        "{}: allocator activity must be identical across warmed requests: {observed:?}",
        case.name
    );
    assert!(
        stats.allocations <= ceiling.allocations
            && stats.deallocations <= ceiling.deallocations
            && stats.reallocations <= ceiling.reallocations
            && stats.bytes_allocated <= ceiling.bytes_allocated
            && stats.bytes_deallocated <= ceiling.bytes_deallocated
            && stats.bytes_reallocated <= ceiling.bytes_reallocated,
        "{}: per-request allocator activity {stats:?} exceeds the committed ceiling {ceiling:?}",
        case.name
    );
}

fn request(case: &Case, address: SocketAddr, response: &mut [u8; 2048]) {
    let used = exchange(address, case.request, response).expect("loopback exchange");
    let reply = &response[..used];
    assert!(
        reply.starts_with(case.status) && contains(reply, case.body),
        "{}: unexpected response",
        case.name
    );
}

/// Earlier exchanges, such as the unmeasured warmup, may still be freeing
/// server-side state. A measured request starts only after the counters have
/// stopped changing, so no earlier free lands in its region.
fn quiescent() {
    let due = Instant::now() + Duration::from_secs(5);
    loop {
        let region = Region::new(&ALLOCATOR);
        std::thread::sleep(Duration::from_millis(50));
        if region.change() == Stats::default() {
            return;
        }
        assert!(
            Instant::now() < due,
            "service allocator activity did not settle"
        );
    }
}

/// The server frees its connection state after the client has read EOF and
/// releases the socket registration on its next driver turn. Wait until every
/// allocation of this request is freed and the counters stay unchanged.
fn released(region: &Region<'_, mimalloc::MiMalloc>) -> Stats {
    let due = Instant::now() + Duration::from_secs(5);
    loop {
        let change = region.change();
        if change.allocations == change.deallocations
            && change.bytes_allocated == change.bytes_deallocated
        {
            std::thread::sleep(Duration::from_millis(20));
            if region.change() == change {
                return change;
            }
        }
        assert!(
            Instant::now() < due,
            "request allocations were not released: {change:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn until_active(address: SocketAddr, active: &[u8], response: &mut [u8; 2048]) {
    let due = Instant::now() + Duration::from_secs(5);
    while !exchange(address, READY.request, response)
        .is_ok_and(|used| contains(&response[..used], active))
    {
        assert!(
            Instant::now() < due,
            "admission did not reach the expected state"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn connect(address: SocketAddr) -> std::io::Result<TcpStream> {
    let socket = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
    socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    socket.set_write_timeout(Some(Duration::from_secs(2)))?;
    Ok(socket)
}

fn exchange(
    address: SocketAddr,
    request: &[u8],
    response: &mut [u8; 2048],
) -> std::io::Result<usize> {
    let mut socket = connect(address)?;
    socket.write_all(request)?;
    let mut used = 0;
    loop {
        let read = socket.read(&mut response[used..])?;
        if read == 0 {
            return Ok(used);
        }
        used += read;
        assert!(used < response.len(), "probe and error responses are small");
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}
