//! Real gateway/child-process integration; this Rust executable acts as a
//! controlled backend when invoked with the Java launch arguments. No shell,
//! Python, model download, external daemon or ignored integration test is used.
use indra_gateway::{commands, configuration::ProbeKind};
use rustix::process::{self, Pid, Signal};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

static DIRECTORY: OnceLock<PathBuf> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(0);

fn main() {
    if std::env::args().any(|argument| argument == "-jar") {
        fake_java();
        return;
    }
    match std::env::args().nth(1).as_deref() {
        Some("-jar") => {
            fake_java();
            return;
        }
        Some("--orphan") => loop {
            thread::park();
        },
        Some("--spawn-orphan") => {
            let child = Command::new(std::env::current_exe().unwrap())
                .arg("--orphan")
                .stdin(Stdio::null())
                .spawn()
                .unwrap();
            let directory = PathBuf::from(std::env::var_os("GATEWAY_TEST_DIR").unwrap());
            fs::write(directory.join("orphan.pid"), child.id().to_string()).unwrap();
            drop(child); // Deliberately adoptable by the gateway subreaper.
            return;
        }
        #[cfg(feature = "sentry")]
        Some("--sentry-correlation-only") => {
            sentry_correlates_failed_exchange_without_otlp();
            return;
        }
        _ => {}
    }
    graceful(Stop::Command);
    graceful(Stop::Signal);
    disconnected_frontend_retains_started_parse();
    truncated_backend_after_headers_is_not_success();
    incomplete_public_headers_expire();
    hung_backend_fails_closed_and_reaps();
    wrong_version_never_becomes_ready();
    lost_ocr_dependency_exits_for_a_restart();
    #[cfg(feature = "sentry")]
    sentry_correlates_failed_exchange_without_otlp();
    dotenv_follows_build_profile();
    completions_and_openapi_need_no_settings();
    println!(
        "gateway process tests passed: command drain, SIGTERM, saturation, admitted upload, client disconnect, timeout, version pin, OCR loss exit, orphan reaping, profile-gated dotenv, completions and OpenAPI"
    );
}

/// The executable under test: the shipped one in image gates, otherwise
/// Cargo's build of the same profile as this test.
fn gateway_binary() -> PathBuf {
    std::env::var_os("INDRA_TEST_GATEWAY_BIN").map_or_else(
        || PathBuf::from(env!("CARGO_BIN_EXE_indra-gateway")),
        PathBuf::from,
    )
}

fn scratch_directory() -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "indra-gateway-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    directory
}

/// Debug builds read `.env` from the working directory and reject a
/// malformed one; release builds never read it. In every gate this test and
/// the gateway it runs are built with the same profile.
fn dotenv_follows_build_profile() {
    let directory = scratch_directory();
    let check = || {
        Command::new(gateway_binary())
            .env_clear()
            .current_dir(&directory)
            .arg("check-config")
            .output()
            .unwrap()
    };
    fs::write(directory.join(".env"), "this is not valid dotenv\n").unwrap();
    let malformed = check();
    fs::write(directory.join(".env"), "TIKA__GATEWAY__MAX_ACTIVE=3\n").unwrap();
    let valid = check();
    fs::remove_dir_all(&directory).unwrap();
    let stdout = String::from_utf8_lossy(&valid.stdout);
    assert!(valid.status.success(), "{stdout}");
    if cfg!(debug_assertions) {
        assert!(!malformed.status.success());
        assert!(
            String::from_utf8_lossy(&malformed.stderr).contains("debug .env could not be parsed")
        );
        assert!(stdout.contains("active=3,"), "{stdout}");
    } else {
        assert!(malformed.status.success());
        assert!(String::from_utf8_lossy(&malformed.stdout).contains("Gateway configuration valid"));
        assert!(stdout.contains("active=2,"), "{stdout}");
    }
}

/// Completions and the API document come from the Clap command and the
/// handlers alone; an invalid lower-priority setting cannot block them.
fn completions_and_openapi_need_no_settings() {
    let directory = scratch_directory();
    let run = |arguments: &[&str]| {
        Command::new(gateway_binary())
            .env_clear()
            .env("TIKA__GATEWAY__MAX_ACTIVE", "invalid")
            .current_dir(&directory)
            .args(arguments)
            .output()
            .unwrap()
    };
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let output = run(&["completions", shell]);
        assert!(output.status.success(), "{shell} completions failed");
        let script = std::str::from_utf8(&output.stdout).unwrap();
        assert!(script.contains("indra-gateway"), "{shell}: {script}");
    }
    let output = run(&["openapi"]);
    fs::remove_dir_all(&directory).unwrap();
    assert!(output.status.success());
    let api: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(api["paths"]["/tika"]["put"]["responses"]["200"].is_object());
    assert!(api["paths"]["/openapi.json"]["get"].is_object());
}

#[cfg(feature = "sentry")]
fn sentry_correlates_failed_exchange_without_otlp() {
    let collector = TcpListener::bind("127.0.0.1:0").unwrap();
    collector.set_nonblocking(true).unwrap();
    let endpoint = collector.local_addr().unwrap();
    let receiver = thread::spawn(move || {
        let due = Instant::now() + Duration::from_secs(8);
        let mut socket = loop {
            match collector.accept() {
                Ok((socket, _)) => break socket,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < due,
                        "Sentry did not export the gateway failure"
                    );
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("Sentry collector accept failed: {error}"),
            }
        };
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut bytes = [0; 65_536];
        let mut used = 0;
        let boundary = loop {
            assert!(used < 8192, "Sentry HTTP header exceeds its bound");
            let count = socket.read(&mut bytes[used..8192]).unwrap();
            assert!(count > 0, "Sentry HTTP header truncated");
            used += count;
            if let Some(at) = bytes[..used].windows(4).position(|s| s == b"\r\n\r\n") {
                break at + 4;
            }
        };
        let headers = std::str::from_utf8(&bytes[..boundary]).unwrap();
        assert!(headers.starts_with("POST /api/1/envelope/ HTTP/1.1\r\n"));
        let length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .expect("fixed Sentry content length");
        let end = boundary.checked_add(length).unwrap();
        assert!(end <= bytes.len() && used <= end, "bounded Sentry envelope");
        while used < end {
            let count = socket.read(&mut bytes[used..end]).unwrap();
            assert!(count > 0, "Sentry envelope truncated");
            used += count;
        }
        let body = &bytes[boundary..end];
        for canary in [b"hello".as_slice(), b"DOCUMENT_SECRET", b"PRIVATE_COOKIE"] {
            assert!(!body.windows(canary.len()).any(|part| part == canary));
        }
        let mut lines = body.splitn(3, |byte| *byte == b'\n');
        let _: serde_json::Value = serde_json::from_slice(lines.next().unwrap()).unwrap();
        let item: serde_json::Value = serde_json::from_slice(lines.next().unwrap()).unwrap();
        assert_eq!(item["type"], "event");
        let event: serde_json::Value = serde_json::from_slice(lines.next().unwrap()).unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        event
    });
    // start_with clears the entire environment and supplies only a Sentry
    // DSN: no OTLP endpoint, propagated parent, or external exporter is enabled.
    let mut gateway = Gateway::start_with(false, Some(endpoint), &[]);
    gateway.ready();
    let mut client = connect(gateway.public);
    client.write_all(b"PUT /fail HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\nAuthorization: DOCUMENT_SECRET\r\nCookie: PRIVATE_COOKIE\r\nConnection: close\r\n\r\nhello").unwrap();
    let response = read_response(&mut client).unwrap();
    assert_eq!(response.status(), 502);
    let request_id = response.request_id();
    assert_eq!(request_id.len(), 16);
    assert!(request_id.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert!(!gateway.wait().success());
    gateway.assert_descendants_reaped();
    assert_eq!(
        gateway.finished_record(request_id)["fields"]["outcome"],
        "tika_failure"
    );
    let event = receiver.join().unwrap();
    assert_eq!(event["tags"]["failure.class"], "tika_failure");
    assert_eq!(event["tags"]["ocr.phase"], "gateway");
    assert_eq!(event["tags"]["request_id"], request_id);
    assert!(event["tags"].get("trace_id").is_none());
    assert!(event["tags"].get("span_id").is_none());
    assert!(event.get("request").is_none());
    assert!(event.get("user").is_none());
}

fn truncated_backend_after_headers_is_not_success() {
    let mut gateway = Gateway::start(false);
    gateway.ready();
    let mut client = connect(gateway.public);
    client.write_all(b"PUT /truncate HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello").unwrap();
    wait_for(
        || gateway.directory.join("started").exists(),
        "truncated backend starts",
    );
    if let Ok(response) = read_response(&mut client)
        && response.used > 0
        && response.status() == 200
    {
        assert!(
            !response.body().ends_with(b"0\r\n\r\n"),
            "truncated chunked response was finalized as successful HTTP"
        );
    }
    assert!(!gateway.wait().success());
    gateway.assert_descendants_reaped();
    assert!(
        fs::read_to_string(gateway.directory.join("gateway.log"))
            .unwrap()
            .contains("tika_failure")
    );
}

fn incomplete_public_headers_expire() {
    let mut gateway = Gateway::start(false);
    gateway.ready();
    let mut socket = connect(gateway.public);
    socket.write_all(b"GET /tika HTTP/1.1\r\n").unwrap();
    let started = Instant::now();
    let mut byte = [0];
    match socket.read(&mut byte) {
        Ok(0) => {}
        Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
        result => panic!("unfinished headers were not closed at their deadline: {result:?}"),
    }
    assert!(started.elapsed() < Duration::from_millis(3500));
    commands::probe(gateway.public, ProbeKind::Ready).unwrap();
    commands::drain(gateway.public).unwrap();
    assert!(gateway.wait().success());
    gateway.assert_descendants_reaped();
}

enum Stop {
    Command,
    Signal,
}

fn graceful(stop: Stop) {
    let mut gateway = Gateway::start(false);
    gateway.ready();
    let mut client = connect(gateway.public);
    client.write_all(b"PUT /tika HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\nConnection: close\r\n\r\nh").unwrap();
    wait_for(|| active(gateway.public, 1), "admitted partial upload");
    // Saturation changes the admission result, not readiness.
    let mut extra = connect(gateway.public);
    extra.write_all(b"PUT /tika HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    let refused = read_response(&mut extra).unwrap();
    assert_eq!(refused.status(), 503);
    commands::probe(gateway.public, ProbeKind::Ready).unwrap();
    match stop {
        Stop::Command => commands::drain(gateway.public).unwrap(),
        Stop::Signal => process::kill_process(
            Pid::from_raw(gateway.child.id() as i32).unwrap(),
            Signal::TERM,
        )
        .unwrap(),
    }
    wait_for(
        || commands::probe(gateway.public, ProbeKind::Ready).is_err(),
        "readiness removed",
    );
    commands::probe(gateway.public, ProbeKind::Health).unwrap();
    gateway.assert_java_alive();
    // The upload admitted before shutdown can finish and start Java afterward.
    client.write_all(b"ello").unwrap();
    wait_for(
        || gateway.directory.join("started").exists(),
        "backend parse started",
    );
    gateway.assert_java_alive();
    fs::write(gateway.directory.join("release"), b"complete").unwrap();
    let response = read_response(&mut client).unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.body(), "日本語 🧪\n".as_bytes());
    assert!(gateway.wait().success());
    gateway.assert_descendants_reaped();
    // Every public request, refused or admitted, leaves one JSON record that
    // carries the X-Request-ID the client received.
    for (wire, outcome) in [(&response, "complete"), (&refused, "saturated")] {
        let record = gateway.finished_record(wire.request_id());
        assert_eq!(record["level"], "INFO");
        assert_eq!(record["fields"]["outcome"], outcome);
        assert!(record["fields"]["elapsed_ms"].is_f64());
    }
}

fn disconnected_frontend_retains_started_parse() {
    let mut gateway = Gateway::start(false);
    gateway.ready();
    let mut client = connect(gateway.public);
    client.write_all(b"PUT /tika HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello").unwrap();
    wait_for(
        || gateway.directory.join("started").exists(),
        "backend before disconnect",
    );
    client.shutdown(Shutdown::Both).unwrap();
    drop(client);
    commands::drain(gateway.public).unwrap();
    assert!(active(gateway.public, 1));
    gateway.assert_java_alive();
    fs::write(gateway.directory.join("release"), b"complete").unwrap();
    assert!(gateway.wait().success());
    gateway.assert_descendants_reaped();
}

fn hung_backend_fails_closed_and_reaps() {
    let mut gateway = Gateway::start(false);
    gateway.ready();
    let mut client = connect(gateway.public);
    client.write_all(b"PUT /hang HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello").unwrap();
    wait_for(
        || gateway.directory.join("started").exists(),
        "hung backend admission",
    );
    // A deadline can close transport before an HTTP error reaches a stalled
    // peer. The required invariant is terminal exit plus no surviving child.
    let _ = read_response(&mut client);
    assert!(!gateway.wait().success());
    gateway.assert_descendants_reaped();
    let log = gateway.log();
    assert!(log.contains("deadline_exceeded"));
    assert!(
        finished_records(&log).any(|record| record["fields"]["outcome"] == "deadline_exceeded"),
        "the timed-out public request has its own finish record"
    );
}

static OCR_STOP: AtomicBool = AtomicBool::new(false);

/// A Compose OCR restart leaves the gateway in a network namespace where OCR
/// never answers again. With the loss bound set, the gateway exits instead of
/// staying unready forever, so the restart policy can rejoin the new one.
fn lost_ocr_dependency_exits_for_a_restart() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let ocr = listener.local_addr().unwrap().to_string();
    let server = thread::spawn(move || {
        while !OCR_STOP.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((socket, _)) => ready_ocr_response(socket),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("OCR fixture accept failed: {error}"),
            }
        }
        // Dropping the listener refuses every later probe, as the old
        // namespace does once its OCR process is gone.
    });
    let mut gateway = Gateway::start_with(
        false,
        None,
        &["--ocr-ready-endpoint", &ocr, "--ocr-loss-exit-seconds", "1"],
    );
    gateway.ready();
    OCR_STOP.store(true, Ordering::Release);
    server.join().unwrap();
    wait_for(
        || commands::probe(gateway.public, ProbeKind::Ready).is_err(),
        "readiness follows the lost dependency",
    );
    assert!(!gateway.wait().success());
    gateway.assert_descendants_reaped();
    let log = gateway.log();
    assert!(
        log.contains("\"failure_class\":\"ocr_dependency_lost\""),
        "{log}"
    );
}

fn ready_ocr_response(mut socket: TcpStream) {
    socket.set_nonblocking(false).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut head = [0; 1024];
    let mut used = 0;
    while !head[..used].ends_with(b"\r\n\r\n") {
        assert!(used < head.len(), "bounded OCR probe request");
        if socket.read_exact(&mut head[used..used + 1]).is_err() {
            return;
        }
        used += 1;
    }
    let body = b"{\"phase\":\"ready\"}";
    let _ = write!(
        socket,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = socket.write_all(body);
}

fn wrong_version_never_becomes_ready() {
    let mut gateway = Gateway::start(true);
    assert!(!gateway.wait().success());
    assert!(commands::probe(gateway.public, ProbeKind::Ready).is_err());
    gateway.assert_descendants_reaped();
    assert!(
        fs::read_to_string(gateway.directory.join("gateway.log"))
            .unwrap()
            .contains("wrong_tika_version")
    );
}

struct Gateway {
    child: Child,
    public: SocketAddr,
    directory: PathBuf,
}
impl Gateway {
    fn start(wrong_version: bool) -> Self {
        Self::start_with(wrong_version, None, &[])
    }
    fn start_with(wrong_version: bool, sentry: Option<SocketAddr>, extra: &[&str]) -> Self {
        let directory = scratch_directory();
        let public = address();
        let backend = address();
        let child = Command::new(gateway_binary())
            .args([
                "--listen",
                &public.to_string(),
                "--backend",
                &backend.to_string(),
            ])
            .arg("--java")
            .arg(std::env::current_exe().unwrap())
            .arg("--jar")
            .arg(directory.join("fixture.jar"))
            .arg("--tika-config")
            .arg(directory.join("fixture.xml"))
            .args([
                "--max-active",
                "1",
                "--max-connections",
                "8",
                "--max-request-bytes",
                "1024",
                "--max-response-bytes",
                "1024",
                "--header-timeout-seconds",
                "1",
                "--upload-timeout-seconds",
                "2",
                "--request-timeout-seconds",
                "2",
                "--client-stall-seconds",
                "1",
                "--startup-timeout-seconds",
                "3",
                "--shutdown-timeout-seconds",
                "4",
                "--child-shutdown-seconds",
                "1",
                "serve",
            ])
            .args(extra)
            .env_clear()
            // The gateway must strip its own TIKA__ settings before spawning Java.
            .env("TIKA__TELEMETRY__ENVIRONMENT", "fixture-inheritance-check")
            .envs(sentry.map(|address| {
                (
                    "TIKA__TELEMETRY__SENTRY_DSN",
                    format!("http://public@{address}/1"),
                )
            }))
            .env("GATEWAY_TEST_DIR", &directory)
            .env(
                "GATEWAY_TEST_WRONG_VERSION",
                if wrong_version { "true" } else { "false" },
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(File::create(directory.join("gateway.log")).unwrap())
            .spawn()
            .unwrap();
        Self {
            child,
            public,
            directory,
        }
    }
    fn ready(&self) {
        wait_for(
            || commands::probe(self.public, ProbeKind::Ready).is_ok(),
            "gateway startup",
        );
        commands::probe(self.public, ProbeKind::Startup).unwrap();
        wait_for(
            || self.directory.join("orphan.pid").exists(),
            "orphan fixture started",
        );
    }
    fn wait(&mut self) -> ExitStatus {
        let due = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < due,
                "gateway did not stop; {}",
                fs::read_to_string(self.directory.join("gateway.log")).unwrap()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn log(&self) -> String {
        fs::read_to_string(self.directory.join("gateway.log")).unwrap()
    }
    /// The one gateway_request_finished record naming `request_id`.
    fn finished_record(&self, request_id: &str) -> serde_json::Value {
        let log = self.log();
        let mut records =
            finished_records(&log).filter(|record| record["fields"]["request_id"] == request_id);
        let record = records
            .next()
            .unwrap_or_else(|| panic!("no finish record for {request_id}: {log}"));
        assert!(records.next().is_none(), "{request_id} finished twice");
        record
    }
    fn assert_java_alive(&self) {
        process::test_kill_process(pid(&self.directory.join("java.pid"))).unwrap();
    }
    fn assert_descendants_reaped(&self) {
        for name in ["java.pid", "orphan.pid"] {
            let file = self.directory.join(name);
            wait_for(|| file.exists(), "child PID recording");
            assert_eq!(
                process::test_kill_process(pid(&file)),
                Err(rustix::io::Errno::SRCH),
                "{name} remains live or zombie after gateway exit"
            );
        }
    }
}
impl Drop for Gateway {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let java = self.directory.join("java.pid");
            if java.exists() {
                let _ = process::kill_process_group(pid(&java), Signal::KILL);
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        fs::remove_dir_all(&self.directory).unwrap();
    }
}

fn pid(path: &Path) -> Pid {
    Pid::from_raw(fs::read_to_string(path).unwrap().trim().parse().unwrap()).unwrap()
}
fn address() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}
fn connect(address: SocketAddr) -> TcpStream {
    let socket = TcpStream::connect_timeout(&address, Duration::from_secs(1)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(6)))
        .unwrap();
    socket
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    socket
}
fn wait_for(mut predicate: impl FnMut() -> bool, description: &str) {
    let due = Instant::now() + Duration::from_secs(8);
    while !predicate() {
        assert!(Instant::now() < due, "timed out: {description}");
        thread::sleep(Duration::from_millis(10));
    }
}
fn active(address: SocketAddr, count: u16) -> bool {
    let mut socket = connect(address);
    socket
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let response = read_response(&mut socket).unwrap();
    serde_json::from_slice::<serde_json::Value>(response.body()).unwrap()["active"] == count
}
struct Wire {
    bytes: [u8; 8192],
    used: usize,
}
impl Wire {
    fn body(&self) -> &[u8] {
        let end = self.bytes[..self.used]
            .windows(4)
            .position(|s| s == b"\r\n\r\n")
            .unwrap();
        &self.bytes[end + 4..self.used]
    }
    fn status(&self) -> u16 {
        std::str::from_utf8(&self.bytes[..self.used])
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap()
    }
    fn request_id(&self) -> &str {
        let end = self.bytes[..self.used]
            .windows(4)
            .position(|s| s == b"\r\n\r\n")
            .unwrap();
        std::str::from_utf8(&self.bytes[..end])
            .unwrap()
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("x-request-id")
                    .then_some(value.trim())
            })
            .expect("gateway correlation response header")
    }
}
fn finished_records(log: &str) -> impl Iterator<Item = serde_json::Value> + '_ {
    log.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|record| record["fields"]["event"] == "gateway_request_finished")
}
fn read_response(socket: &mut TcpStream) -> io::Result<Wire> {
    let mut wire = Wire {
        bytes: [0; 8192],
        used: 0,
    };
    loop {
        assert!(
            wire.used < wire.bytes.len(),
            "test response exceeds fixed storage"
        );
        let count = socket.read(&mut wire.bytes[wire.used..])?;
        if count == 0 {
            return Ok(wire);
        }
        wire.used += count;
    }
}

fn fake_java() {
    if std::env::vars_os().any(|(key, _)| key.as_encoded_bytes().starts_with(b"TIKA__")) {
        eprintln!("fake java inherited a TIKA__ gateway variable");
        std::process::exit(86);
    }
    let directory =
        DIRECTORY.get_or_init(|| PathBuf::from(std::env::var_os("GATEWAY_TEST_DIR").unwrap()));
    fs::write(directory.join("java.pid"), std::process::id().to_string()).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .arg("--spawn-orphan")
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    drop(child);
    wait_for(
        || directory.join("orphan.pid").exists(),
        "orphan fixture before version readiness",
    );
    let mut args = std::env::args();
    let mut port = None;
    while let Some(argument) = args.next() {
        if argument == "--port" {
            port = args.next();
        }
    }
    let listener = TcpListener::bind(("127.0.0.1", port.unwrap().parse::<u16>().unwrap())).unwrap();
    let mut workers: [Option<thread::JoinHandle<()>>; 16] = std::array::from_fn(|_| None);
    for stream in listener.incoming() {
        let socket = stream.unwrap();
        for worker in &mut workers {
            if worker.as_ref().is_some_and(thread::JoinHandle::is_finished) {
                worker.take().unwrap().join().unwrap();
            }
        }
        if let Some(worker) = workers.iter_mut().find(|worker| worker.is_none()) {
            *worker = Some(thread::spawn(move || fake_response(socket, directory)));
        }
    }
}

fn fake_response(mut socket: TcpStream, directory: &Path) {
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut headers = [0; 8192];
    let mut used = 0;
    while !headers[..used].ends_with(b"\r\n\r\n") {
        assert!(used < headers.len());
        if socket.read_exact(&mut headers[used..used + 1]).is_err() {
            return;
        }
        used += 1;
    }
    let value = std::str::from_utf8(&headers[..used]).unwrap();
    let path = value.split_whitespace().nth(1).unwrap();
    if path == "/version" {
        let text = if std::env::var("GATEWAY_TEST_WRONG_VERSION").unwrap() == "true" {
            "Apache Tika 0.0.0"
        } else {
            "Apache Tika 3.3.2"
        };
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
            text.len()
        )
        .unwrap();
        return;
    }
    let length = value
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .map(|(_, value)| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    let mut body = [0; 1024];
    assert!(length <= body.len());
    socket.read_exact(&mut body[..length]).unwrap();
    assert_eq!(&body[..length], b"hello");
    fs::write(directory.join("started"), b"started").unwrap();
    if path == "/fail" {
        // An actual upstream EOF exercises the production exchange failure and
        // its first contextual report, before supervisor child termination.
        return;
    }
    if path == "/truncate" {
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nabc").unwrap();
        return;
    }
    while path == "/hang" || !directory.join("release").exists() {
        thread::sleep(Duration::from_millis(5));
    }
    let response = "日本語 🧪\n";
    write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len()).unwrap();
    socket.write_all(response.as_bytes()).unwrap();
}
