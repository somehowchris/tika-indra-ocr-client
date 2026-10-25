//! Under `panic = "abort"` the panic hook is the only crash report. This test
//! re-executes itself: the child initializes telemetry against a local Sentry
//! listener and panics on a thread with a hostile name; the parent checks the
//! child's stderr and the envelope it sent. Test builds unwind, so the child
//! aborts explicitly once the panicking thread has ended and never reaches
//! the orderly telemetry shutdown: only the hook can have flushed the report.
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::process::ExitStatusExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Quote, backslash, newline, a C0 control and non-ASCII text; thread names
/// cannot contain NUL.
static THREAD: &str = "ocr\"worker\\\n\u{1} Zürich";
static SENTINEL: &str = "SECRET-DOCUMENT-PAYLOAD";

fn main() {
    match std::env::var("INDRA_PANIC_PROBE_DSN") {
        Ok(dsn) => panic_in_child(dsn),
        Err(_) => parent(),
    }
}

fn panic_in_child(dsn: String) -> ! {
    let options = indra_telemetry::Options {
        sentry_dsn: Some(dsn),
        export_timeout_ms: 2000,
        ..indra_telemetry::Options::default()
    };
    let _guard = indra_telemetry::initialize(&options).expect("telemetry setup");
    let worker = std::thread::Builder::new()
        .name(String::from(THREAD))
        .spawn(|| panic!("{SENTINEL}"))
        .expect("named thread");
    assert!(worker.join().is_err(), "the worker panicked");
    std::process::abort()
}

fn parent() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("local Sentry listener");
    listener.set_nonblocking(true).expect("nonblocking accept");
    let address = listener.local_addr().expect("listener address");
    let child = Command::new(std::env::current_exe().expect("test executable"))
        .env(
            "INDRA_PANIC_PROBE_DSN",
            format!("http://public@{address}/1"),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("child process");
    let deadline = Instant::now() + Duration::from_secs(10);
    let socket = loop {
        match listener.accept() {
            Ok((socket, _)) => break socket,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "no Sentry envelope arrived");
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(error) => panic!("Sentry listener accept: {error}"),
        }
    };
    let envelope = receive(socket);
    let output = child.wait_with_output().expect("child exit");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert_eq!(
        output.status.signal(),
        Some(6),
        "the child aborts after the hook\n{stderr}"
    );
    let mut panics = 0;
    for line in stderr.lines() {
        let record: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|_| panic!("stderr carries only JSON lines\n{stderr}"));
        if record["fields"]["event"] == "panic" {
            panics += 1;
            assert_eq!(record["level"], "ERROR");
            assert_eq!(record["target"], "indra_telemetry");
            assert_eq!(record["fields"]["thread"], THREAD);
            assert!(
                record["fields"]["file"]
                    .as_str()
                    .is_some_and(|file| file.ends_with("panic_hook.rs")),
                "{line}"
            );
        }
    }
    assert_eq!(panics, 1, "one synchronous panic line\n{stderr}");
    assert!(
        !stderr.contains(SENTINEL),
        "the payload never reaches stderr\n{stderr}"
    );
    assert!(
        !envelope.contains(SENTINEL),
        "the payload never reaches Sentry"
    );
    let event = envelope
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|item| item.get("exception").is_some())
        .expect("envelope carries an error event");
    assert_eq!(event["tags"]["failure.class"], "panic");
    assert_eq!(event["tags"]["ocr.phase"], "process");
    assert_eq!(event["exception"]["values"][0]["type"], "panic");
    println!("panic hook: one JSON stderr line and a flushed Sentry panic event, no payload");
}

/// Reads one bounded HTTP request and returns its body.
fn receive(mut socket: TcpStream) -> String {
    socket.set_nonblocking(false).expect("blocking exchange");
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    let mut data = [0_u8; 65_536];
    let mut used = 0;
    let boundary = loop {
        let read = socket.read(&mut data[used..]).expect("request bytes");
        assert!(read > 0 && used + read < data.len(), "bounded request");
        used += read;
        if let Some(position) = data[..used]
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
        {
            break position + 4;
        }
    };
    let headers = std::str::from_utf8(&data[..boundary]).expect("ASCII headers");
    assert!(headers.starts_with("POST /api/1/envelope/ "), "{headers}");
    let length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().expect("length"))
        })
        .expect("fixed content length");
    let end = boundary + length;
    assert!(end <= data.len(), "bounded envelope");
    while used < end {
        let read = socket.read(&mut data[used..end]).expect("envelope bytes");
        assert!(read > 0, "complete envelope");
        used += read;
    }
    socket
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        .expect("acknowledgement");
    String::from(std::str::from_utf8(&data[boundary..end]).expect("UTF-8 envelope"))
}
