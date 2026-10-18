//! Separate real-process gate: these tests require linked native libraries and
//! the immutable local model snapshot. Router tests cannot prove worker startup
//! or operating-system signal handling.

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

static IMAGE: &[u8] = include_bytes!("../../../../backend/tests/data/smoke.png");
static DEFAULT_BINARY: &str = env!("CARGO_BIN_EXE_indra-server");

enum Stop {
    DrainCommand,
    Sigterm,
}

struct Process(Child);

impl Drop for Process {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[test]
#[ignore = "requires real native linkage and INDRA_TEST_MODEL_DIR; executable/thread overrides support scratch containers"]
fn real_startup_drain_and_sigterm_complete_previously_admitted_ocr() {
    let binary = std::env::var_os("INDRA_TEST_SERVER_BIN")
        .map_or_else(|| PathBuf::from(DEFAULT_BINARY), PathBuf::from);
    let threads = match std::env::var("INDRA_TEST_THREADS") {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => String::from("1"),
        Err(_) => panic!("INDRA_TEST_THREADS must be valid UTF-8"),
    };
    assert!(
        matches!(threads.as_str(), "1" | "2"),
        "INDRA_TEST_THREADS must be exactly 1 or 2"
    );
    println!(
        "native_lifecycle_settings={{\"workers\":1,\"threads\":{threads},\"cv_threads\":{threads}}}"
    );
    let model_dir = PathBuf::from(
        std::env::var_os("INDRA_TEST_MODEL_DIR")
            .expect("point INDRA_TEST_MODEL_DIR at the verified pinned snapshot"),
    );
    for stop in [Stop::DrainCommand, Stop::Sigterm] {
        let listener = TcpListener::bind("127.0.0.1:0").expect("reserve test address");
        let address = listener.local_addr().expect("test address");
        drop(listener);
        let mut process = Process(
            Command::new(&binary)
                .env_clear()
                .env("OMP_NUM_THREADS", &threads)
                .env("OPENBLAS_NUM_THREADS", &threads)
                .args(["--listen", &address.to_string(), "--model-dir"])
                .arg(&model_dir)
                .args([
                    "--threads",
                    &threads,
                    "--cv-threads",
                    &threads,
                    "--log-filter",
                    "warn",
                    "--startup-timeout-seconds",
                    "300",
                    "serve",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .expect("native service process"),
        );
        until(Duration::from_secs(300), || {
            assert!(
                process.0.try_wait().expect("startup status").is_none(),
                "worker must warm successfully"
            );
            exchange(address, "/readyz")
                .is_some_and(|response| response.starts_with("HTTP/1.1 200 "))
        });
        assert!(
            exchange(address, "/startupz")
                .expect("startup probe")
                .starts_with("HTTP/1.1 200 ")
        );

        // Hold an admitted upload open. This deterministically covers a request
        // accepted before drain that only reaches the native FIFO afterward.
        let mut upload = connect(address).expect("OCR connection");
        write!(upload, "POST /ocr HTTP/1.1\r\nHost: localhost\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", IMAGE.len()).expect("request headers");
        upload.write_all(&IMAGE[..1]).expect("first image byte");
        until(Duration::from_secs(3), || {
            exchange(address, "/readyz").is_some_and(|response| response.contains("\"active\":1"))
        });

        match stop {
            Stop::DrainCommand => assert!(
                Command::new(&binary)
                    .env_clear()
                    .args(["--listen", &address.to_string(), "drain"])
                    .status()
                    .expect("shell-free drain command")
                    .success()
            ),
            Stop::Sigterm => rustix::process::kill_process(
                rustix::process::Pid::from_raw(process.0.id() as i32).expect("owned child PID"),
                rustix::process::Signal::TERM,
            )
            .expect("send SIGTERM directly without a shell or external binary"),
        }
        until(Duration::from_secs(3), || {
            exchange(address, "/readyz").is_some_and(|response| {
                response.starts_with("HTTP/1.1 503 ") && response.contains("\"phase\":\"draining\"")
            })
        });
        assert!(
            exchange(address, "/healthz")
                .expect("health during drain")
                .starts_with("HTTP/1.1 200 ")
        );
        upload
            .write_all(&IMAGE[1..])
            .expect("complete the accepted image");
        let response = receive(upload).expect("accepted native OCR response");
        assert!(
            response.starts_with("HTTP/1.1 200 "),
            "accepted work must finish: {response}"
        );
        let (_, text) = response.split_once("\r\n\r\n").expect("response framing");
        assert_eq!(text, "INVOICE 12345", "real native smoke text is exact");
        until(Duration::from_secs(5), || {
            process
                .0
                .try_wait()
                .expect("drained process status")
                .is_some_and(|status| {
                    assert!(status.success(), "drain must exit successfully");
                    true
                })
        });
    }
}

fn until(timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "native lifecycle acceptance deadline"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn connect(address: SocketAddr) -> std::io::Result<TcpStream> {
    let socket = TcpStream::connect_timeout(&address, Duration::from_secs(1))?;
    socket.set_read_timeout(Some(Duration::from_secs(60)))?;
    socket.set_write_timeout(Some(Duration::from_secs(2)))?;
    Ok(socket)
}

fn exchange(address: SocketAddr, path: &'static str) -> Option<String> {
    let mut socket = connect(address).ok()?;
    socket.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    write!(
        socket,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    receive(socket).ok()
}

fn receive(socket: TcpStream) -> std::io::Result<String> {
    let mut response = String::with_capacity(1024);
    socket
        .take(4 * 1024 * 1024 + 16 * 1024)
        .read_to_string(&mut response)?;
    Ok(response)
}
