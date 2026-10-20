//! The real adapter executable against controlled loopback HTTP peers: request
//! framing, UTF-8 output, failure modes that must leave no output file, the
//! total deadline, and the readiness probes. No OCR is measured. Set
//! `BRIDGE_BIN` to test a specific executable, such as the static release ELF.
use std::{
    fs::{self, File},
    io::{BufReader, ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tika_indra_ocr_client::bounded_line;

const PAYLOAD_BYTES: usize = 256 * 512;

struct Capture {
    first_line: String,
    body: Vec<u8>,
}

enum Reply {
    /// A complete `text/plain` 200 response with an exact Content-Length.
    Http(&'static [u8]),
    /// Raw bytes, exactly as given.
    Bytes(&'static [u8]),
    /// Headers announcing 20 bytes, then one byte every 200 ms.
    Drip,
    /// Read the request, then close without a response.
    Close,
}

impl Reply {
    fn send(&self, socket: &mut TcpStream) {
        let result = match self {
            Self::Http(body) => {
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket
                    .write_all(head.as_bytes())
                    .and_then(|()| socket.write_all(body))
            }
            Self::Bytes(bytes) => socket.write_all(bytes),
            Self::Drip => socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 20\r\n\r\n",
                )
                .and_then(|()| {
                    for _ in 0..20 {
                        std::thread::sleep(Duration::from_millis(200));
                        socket.write_all(b"x")?;
                    }
                    Ok(())
                }),
            Self::Close => Ok(()),
        };
        // A client that has already hung up (deadline tests) is expected.
        drop(result);
    }
}

fn serve(socket: TcpStream, reply: &Reply, captures: &Mutex<Vec<Capture>>) {
    // BSD-derived kernels let an accepted socket inherit the listener's
    // non-blocking flag; the peer must block with bounded timeouts instead.
    socket.set_nonblocking(false).expect("blocking peer socket");
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("peer read timeout");
    socket
        .set_write_timeout(Some(Duration::from_secs(2)))
        .expect("peer write timeout");
    let mut reader = BufReader::new(socket);
    let mut line = [0; 8192];
    let mut first = String::new();
    let mut length = 0;
    let mut fields = 0;
    loop {
        fields += 1;
        assert!(fields <= 64, "bounded request head");
        let count = match bounded_line(&mut reader, &mut line) {
            Ok(count) => count,
            Err(_) => return,
        };
        if count == 0 {
            return; // peer closed before a complete head
        }
        let raw = &line[..count];
        if first.is_empty() {
            first = String::from_utf8_lossy(raw).trim_end().to_string();
            continue;
        }
        if raw == b"\r\n" {
            break;
        }
        let text = String::from_utf8_lossy(raw);
        if let Some((key, value)) = text.split_once(':')
            && key.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().expect("content length");
        }
    }
    let mut body = vec![0; length];
    if reader.read_exact(&mut body).is_err() {
        return;
    }
    captures.lock().expect("capture mutex").push(Capture {
        first_line: first,
        body,
    });
    reply.send(reader.get_mut());
}

/// Serve `reply` on a fresh loopback port while `test` runs; return its result
/// and every request the peer received.
fn with_peer<T>(reply: Reply, test: impl FnOnce(&str) -> T) -> (T, Vec<Capture>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback peer");
    listener.set_nonblocking(true).expect("non-blocking accept");
    let address = listener.local_addr().expect("peer address").to_string();
    let stop = AtomicBool::new(false);
    let captures = Mutex::new(Vec::new());
    let result = std::thread::scope(|scope| {
        scope.spawn(|| {
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((socket, _)) => serve(socket, &reply, &captures),
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("peer accept: {error}"),
                }
            }
        });
        let result = test(&address);
        stop.store(true, Ordering::Release);
        result
    });
    (result, captures.into_inner().expect("capture mutex"))
}

fn bridge() -> PathBuf {
    std::env::var_os("BRIDGE_BIN").map_or_else(
        || PathBuf::from(env!("CARGO_BIN_EXE_indra-tesseract")),
        PathBuf::from,
    )
}

fn run(mut command: Command, timeout: Duration) -> Output {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("actual bridge executable");
    let due = Instant::now() + timeout;
    loop {
        if child.try_wait().expect("poll owned child").is_some() {
            break;
        }
        if Instant::now() >= due {
            child.kill().expect("kill stalled bridge");
            child.wait().expect("reap stalled bridge");
            panic!("bridge exceeded {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    child.wait_with_output().expect("bridge output")
}

struct Fixture {
    directory: PathBuf,
    input: PathBuf,
    base: PathBuf,
    output: PathBuf,
    payload: Vec<u8>,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "indra bridge {name} {} {}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir(&directory).expect("exclusive test directory");
        let input = directory.join("page with spaces.png");
        let base = directory.join("output base");
        let output = directory.join("output base.txt");
        let payload: Vec<u8> = (0..=255).cycle().take(PAYLOAD_BYTES).collect();
        fs::write(&input, &payload).expect("owned test input");
        Self {
            directory,
            input,
            base,
            output,
            payload,
        }
    }

    fn run(&self, address: &str, extra: &[(&str, &str)]) -> Output {
        let mut command = Command::new(bridge());
        command
            .arg(&self.input)
            .arg(&self.base)
            .args([
                "--psm",
                "3",
                "-c",
                "page_separator=",
                "-c",
                "preserve_interword_spaces=0",
                "txt",
            ])
            .env("INDRA_ADDR", address);
        for (key, value) in extra {
            command.env(key, value);
        }
        run(command, Duration::from_secs(5))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn probe(args: &[&str], env: &[(&str, &str)], timeout: Duration) -> Output {
    let mut command = Command::new(bridge());
    command.args(args);
    for (key, value) in env {
        command.env(key, value);
    }
    run(command, timeout)
}

fn stderr_contains(output: &Output, needle: &str) -> bool {
    String::from_utf8_lossy(&output.stderr).contains(needle)
}

#[test]
fn complete_payload_and_utf8_output() {
    let fixture = Fixture::new("payload");
    let expected = "Zürich — café. 日本語\n".as_bytes();
    let (output, captures) = with_peer(Reply::Http(expected), |address| fixture.run(address, &[]));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&fixture.output).expect("output file"), expected);
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].first_line, "POST /ocr HTTP/1.1");
    assert_eq!(captures[0].body, fixture.payload);
}

#[test]
fn backend_failure_never_creates_output() {
    static REPLIES: [Reply; 4] = [
        Reply::Bytes(b"HTTP/1.1 503 Busy\r\nContent-Length: 0\r\n\r\n"),
        Reply::Bytes(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 100\r\n\r\nshort",
        ),
        Reply::Bytes(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 4294967296\r\n\r\n",
        ),
        Reply::Http(b"\xff\xfe"),
    ];
    for (index, reply) in REPLIES.iter().enumerate() {
        let fixture = Fixture::new(&format!("failure-{index}"));
        let reply = match reply {
            Reply::Bytes(bytes) => Reply::Bytes(bytes),
            Reply::Http(body) => Reply::Http(body),
            Reply::Drip => Reply::Drip,
            Reply::Close => Reply::Close,
        };
        let (output, _) = with_peer(reply, |address| fixture.run(address, &[]));
        assert!(!output.status.success(), "reply {index} must fail");
        assert!(
            !fixture.output.exists(),
            "reply {index} must not create output"
        );
    }
}

#[test]
fn total_deadline_cannot_be_extended_by_a_drip_response() {
    let fixture = Fixture::new("drip");
    let started = Instant::now();
    let (output, _) = with_peer(Reply::Drip, |address| {
        fixture.run(address, &[("INDRA_TIMEOUT_SECONDS", "1")])
    });
    let elapsed = started.elapsed();
    assert!(!output.status.success());
    assert!(
        elapsed < Duration::from_millis(1600),
        "deadline not enforced: {elapsed:?}"
    );
    assert!(!fixture.output.exists());
}

#[test]
fn existing_output_is_not_overwritten() {
    let fixture = Fixture::new("keep");
    fs::write(&fixture.output, "keep this").expect("existing output");
    let (output, _) = with_peer(Reply::Http(b"replacement"), |address| {
        fixture.run(address, &[])
    });
    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(&fixture.output).expect("kept output"),
        "keep this"
    );
}

#[test]
fn valid_blank_page_is_a_successful_empty_file() {
    let fixture = Fixture::new("blank");
    let (output, _) = with_peer(Reply::Http(b""), |address| fixture.run(address, &[]));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&fixture.output).expect("empty output"), b"");
}

#[test]
fn availability_probe_does_not_require_a_backend() {
    for args in [&[][..], &["--version"][..]] {
        let output = probe(args, &[], Duration::from_secs(2));
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("indra-tesseract"));
    }
}

#[test]
fn health_calls_the_ready_endpoint() {
    let (output, captures) = with_peer(Reply::Http(b"ready\n"), |address| {
        probe(
            &["--ready"],
            &[("INDRA_ADDR", address)],
            Duration::from_secs(3),
        )
    });
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(captures[0].first_line, "GET /readyz HTTP/1.1");
}

#[test]
fn oversized_input_fails_before_the_network() {
    let fixture = Fixture::new("oversized");
    File::options()
        .write(true)
        .open(&fixture.input)
        .expect("input")
        .set_len(20 * 1024 * 1024 + 1)
        .expect("sparse oversized input");
    let output = fixture.run("127.0.0.1:1", &[]);
    assert!(!output.status.success());
    assert!(stderr_contains(&output, "20 MiB"));
    assert!(!fixture.output.exists());
}

#[test]
fn combined_health_requires_tika_too() {
    for (tika_reply, expected) in [
        (
            Reply::Bytes(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n"),
            0,
        ),
        (Reply::Bytes(b"HTTP/1.1 503 Busy\r\n\r\n"), 1),
    ] {
        let (output, _) = with_peer(Reply::Http(b"ready\n"), |indra| {
            let (output, _) = with_peer(tika_reply, |tika| {
                probe(
                    &["--tika-ready"],
                    &[("INDRA_ADDR", indra), ("TIKA_ADDR", tika)],
                    Duration::from_secs(5),
                )
            });
            output
        });
        assert_eq!(
            output.status.code(),
            Some(expected),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn tika_server_health_does_not_require_indra() {
    for (reply, expected) in [
        (
            Reply::Bytes(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n"),
            0,
        ),
        (Reply::Bytes(b"HTTP/1.1 503 Busy\r\n\r\n"), 1),
        (Reply::Bytes(b"not HTTP\r\n"), 1),
        (Reply::Close, 1),
    ] {
        let (output, captures) = with_peer(reply, |tika| {
            probe(
                &["--tika-server-ready"],
                &[("INDRA_ADDR", "deliberately-invalid"), ("TIKA_ADDR", tika)],
                Duration::from_secs(3),
            )
        });
        assert_eq!(
            output.status.code(),
            Some(expected),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(captures.len(), 1);
        assert_eq!(captures[0].first_line, "GET /version HTTP/1.1");
        assert!(captures[0].body.is_empty());
    }
}

#[test]
fn remote_address_is_rejected() {
    let fixture = Fixture::new("remote");
    let output = fixture.run("192.0.2.1:8081", &[]);
    assert!(!output.status.success());
    assert!(stderr_contains(&output, "loopback"));
}
