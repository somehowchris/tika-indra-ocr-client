//! The experimental worker must retain both admitted requests through drain.
//! Shared-call assertions inspect content-free events emitted only after a real
//! native Session::run, in addition to verifying both complete response bodies.

use std::fs::{File, OpenOptions};

use super::*;

static TIFF: &[u8] = include_bytes!("../../../../../backend/tests/data/two-pages.tiff");
static EXPECTED: [&str; 2] = ["INVOICE 12345\n\u{c}\nSECOND PAGE 67890", "INVOICE 12345"];
static LOG_LIMIT: u64 = 4 * 1024 * 1024;

struct Logs(PathBuf);

impl Logs {
    fn create(address: SocketAddr, mode: &str) -> (Self, File) {
        let path = std::env::temp_dir().join(format!(
            "indra-continuous-{}-{}-{mode}.jsonl",
            std::process::id(),
            address.port()
        ));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("create an exclusively owned native log file");
        (Self(path), file)
    }

    fn verify(&self) {
        let mut text = String::with_capacity(LOG_LIMIT as usize);
        File::open(&self.0)
            .expect("native JSON log")
            .take(LOG_LIMIT + 1)
            .read_to_string(&mut text)
            .expect("bounded native log read");
        assert!(
            text.len() <= LOG_LIMIT as usize,
            "native acceptance log exceeds its fixed budget"
        );
        let mut shared = [0_usize; 2];
        for line in text.lines() {
            let record: serde_json::Value =
                serde_json::from_str(line).expect("native logs are JSON objects");
            let fields = &record["fields"];
            if fields["event"] == "continuous_invocation" && fields["shared"] == true {
                assert!(
                    fields["owner_zero_rows"]
                        .as_u64()
                        .is_some_and(|rows| rows > 0)
                );
                assert!(
                    fields["owner_one_rows"]
                        .as_u64()
                        .is_some_and(|rows| rows > 0)
                );
                match fields["model"].as_str() {
                    Some("script") => shared[0] += 1,
                    Some("rec") => shared[1] += 1,
                    Some("rec_cjk") => {}
                    _ => panic!("unrecognized native stateless model label"),
                }
            }
        }
        // Preserve the complete bounded process log in the CI test artifact.
        println!("{text}");
        assert!(
            shared.iter().all(|calls| *calls > 0),
            "both owners must participate in actual script and recognizer calls: {shared:?}"
        );
        println!(
            "continuous_lifecycle_shared_calls={{\"script\":{},\"rec\":{}}}",
            shared[0], shared[1]
        );
    }
}

impl Drop for Logs {
    fn drop(&mut self) {
        if std::thread::panicking()
            && let Ok(file) = File::open(&self.0)
        {
            let mut text = String::with_capacity(LOG_LIMIT as usize);
            if file.take(LOG_LIMIT).read_to_string(&mut text).is_ok() {
                eprintln!("native pilot failure log (bounded to {LOG_LIMIT} bytes):\n{text}");
            }
        }
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
#[ignore = "requires real native linkage and INDRA_TEST_MODEL_DIR; run at actual one/two CPU caps"]
fn two_admitted_owners_share_native_calls_and_both_finish_during_drain() {
    let binary = std::env::var_os("INDRA_TEST_SERVER_BIN")
        .map_or_else(|| PathBuf::from(DEFAULT_BINARY), PathBuf::from);
    let threads = match std::env::var("INDRA_TEST_THREADS") {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => String::from("1"),
        Err(_) => panic!("INDRA_TEST_THREADS must be valid UTF-8"),
    };
    assert!(matches!(threads.as_str(), "1" | "2"));
    let models = PathBuf::from(
        std::env::var_os("INDRA_TEST_MODEL_DIR").expect("explicit immutable model snapshot"),
    );
    for stop in [Stop::DrainCommand, Stop::Sigterm] {
        let mode = match stop {
            Stop::DrainCommand => "drain",
            Stop::Sigterm => "sigterm",
        };
        let listener = TcpListener::bind("127.0.0.1:0").expect("reserve native test address");
        let address = listener.local_addr().expect("test address");
        drop(listener);
        let (logs, writer) = Logs::create(address, mode);
        let mut process = Process(
            Command::new(&binary)
                .env_clear()
                .env("RUST_MIN_STACK", "2097152")
                .env("OMP_NUM_THREADS", &threads)
                .env("OPENBLAS_NUM_THREADS", &threads)
                .args(["--listen", &address.to_string(), "--model-dir"])
                .arg(&models)
                .args([
                    "--threads",
                    &threads,
                    "--cv-threads",
                    &threads,
                    "--experimental-continuous-batching",
                    "true",
                    "--workers",
                    "1",
                    "--queue-capacity",
                    "2",
                    "--batch-size",
                    "64",
                    "--math",
                    "false",
                    "--log-filter",
                    "info",
                    "--startup-timeout-seconds",
                    "300",
                    "serve",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::from(writer))
                .spawn()
                .expect("experimental native service"),
        );
        until(Duration::from_secs(300), || {
            assert!(
                process.0.try_wait().expect("startup status").is_none(),
                "native worker starts"
            );
            exchange(address, "/readyz").is_some_and(|r| r.starts_with("HTTP/1.1 200 "))
        });
        let inputs = [TIFF, IMAGE];
        let mut uploads = std::array::from_fn::<_, 2, _>(|owner| {
            let mut socket = connect(address).expect("admitted request socket");
            write!(socket, "POST /ocr HTTP/1.1\r\nHost: localhost\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                if owner == 0 { "image/tiff" } else { "image/png" }, inputs[owner].len()).expect("OCR headers");
            socket
                .write_all(&inputs[owner][..1])
                .expect("hold an admitted upload");
            socket
        });
        until(Duration::from_secs(3), || {
            exchange(address, "/readyz").is_some_and(|r| r.contains("\"active\":2"))
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
                rustix::process::Pid::from_raw(
                    i32::try_from(process.0.id()).expect("owned PID fits"),
                )
                .expect("owned child PID"),
                rustix::process::Signal::TERM,
            )
            .expect("SIGTERM to the owned native process"),
        }
        until(Duration::from_secs(3), || {
            exchange(address, "/readyz").is_some_and(|r| {
                r.starts_with("HTTP/1.1 503 ")
                    && r.contains("\"phase\":\"draining\"")
                    && r.contains("\"active\":2")
            })
        });
        assert!(
            exchange(address, "/healthz")
                .expect("health during drain")
                .starts_with("HTTP/1.1 200 ")
        );
        for (upload, input) in uploads.iter_mut().zip(inputs) {
            upload
                .write_all(&input[1..])
                .expect("finish both previously admitted bodies");
        }
        for (owner, upload) in uploads.into_iter().enumerate() {
            let response = receive(upload).expect("complete admitted response");
            assert!(
                response.starts_with("HTTP/1.1 200 "),
                "owner {owner} response: {response}"
            );
            let (_, body) = response.split_once("\r\n\r\n").expect("response framing");
            assert_eq!(
                body, EXPECTED[owner],
                "each complete response retains its owner and TIFF page order"
            );
        }
        until(Duration::from_secs(5), || {
            process
                .0
                .try_wait()
                .expect("terminal child state")
                .is_some_and(|status| {
                    assert!(status.success(), "native drain exits cleanly");
                    true
                })
        });
        logs.verify();
        println!(
            "continuous_lifecycle={{\"stop\":\"{mode}\",\"threads\":{threads},\"owners\":2,\"tiff_frames\":2,\"clean_exit\":true}}"
        );
    }
}
