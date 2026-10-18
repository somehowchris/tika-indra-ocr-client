//! Actual CLI subprocess environment -> bounded HTTP headers. No global env mutation.
use std::{
    fs,
    io::{BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tika_indra_ocr_client::bounded_line;

struct Case {
    parent: Option<&'static str>,
    state: Option<&'static str>,
    expected_parent: Option<&'static str>,
    expected_state: Option<&'static str>,
}

static FIRST: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
static SECOND: &str = "00-abcd2f3577b34da6a3ce929d0e0e4736-11f067aa0ba902b7-01";
static CASES: [Case; 6] = [
    Case {
        parent: Some(FIRST),
        state: Some("a=one,b=two"),
        expected_parent: Some(FIRST),
        expected_state: Some("a=one,b=two"),
    },
    Case {
        parent: Some(SECOND),
        state: Some("vendor=other"),
        expected_parent: Some(SECOND),
        expected_state: Some("vendor=other"),
    },
    Case {
        parent: None,
        state: Some("vendor=orphan"),
        expected_parent: None,
        expected_state: None,
    },
    Case {
        parent: Some("invalid"),
        state: Some("vendor=orphan"),
        expected_parent: None,
        expected_state: None,
    },
    Case {
        parent: Some(FIRST),
        state: Some("a=one,a=two"),
        expected_parent: Some(FIRST),
        expected_state: None,
    },
    Case {
        parent: Some(SECOND),
        state: Some("vendor=one\r\nBaggage: private"),
        expected_parent: Some(SECOND),
        expected_state: None,
    },
];

fn spawn(work: &Path, index: usize, address: std::net::SocketAddr) -> Child {
    let case = &CASES[index];
    let input = work.join(format!("{index}.input"));
    fs::write(&input, [index as u8]).expect("owned test input");
    let mut command = Command::new(env!("CARGO_BIN_EXE_indra-tesseract"));
    command
        .arg(&input)
        .arg(work.join(format!("{index}.out")))
        .args([
            "--psm",
            "3",
            "-c",
            "page_separator=",
            "-c",
            "preserve_interword_spaces=0",
            "txt",
        ])
        .env("INDRA_ADDR", address.to_string())
        .env("INDRA_TIMEOUT_SECONDS", "2")
        .env_remove("INDRA_TRACEPARENT")
        .env_remove("INDRA_TRACESTATE")
        .env("BAGGAGE", "private")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    if let Some(parent) = case.parent {
        command.env("INDRA_TRACEPARENT", parent);
    }
    if let Some(state) = case.state {
        command.env("INDRA_TRACESTATE", state);
    }
    command.spawn().expect("actual bridge executable")
}

fn wait(child: &mut Child) {
    let due = Instant::now() + Duration::from_secs(4);
    loop {
        if let Some(status) = child.try_wait().expect("poll owned child") {
            assert!(
                status.success(),
                "CLI succeeds with valid transport even when trace context is invalid"
            );
            return;
        }
        if Instant::now() >= due {
            child.kill().expect("kill stalled test child");
            child.wait().expect("reap stalled child");
            panic!("bridge exceeded its bounded transport deadline");
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn concurrent_cli_processes_forward_only_their_valid_request_context() {
    let work = std::env::temp_dir().join(format!(
        "indra-trace-wire-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&work).expect("exclusive test directory");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bounded local peer");
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::scope(|scope| {
        let server = scope.spawn(|| {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut seen = [false; CASES.len()];
            let mut count = 0;
            while count < CASES.len() {
                assert!(Instant::now() < deadline, "finite test accept budget");
                let (socket, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("peer accept: {error}"),
                };
                socket.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
                socket.set_write_timeout(Some(Duration::from_secs(1))).unwrap();
                let mut reader = BufReader::new(socket);
                let mut line = [0; 8192];
                let mut parent = [0; 55];
                let mut parent_length = 0;
                let mut state = [0; 512];
                let mut state_length = 0;
                let mut fields = 0;
                loop {
                    fields += 1;
                    assert!(fields <= 16, "bounded HTTP fields");
                    let length = bounded_line(&mut reader, &mut line).unwrap();
                    assert!(line[..length].ends_with(b"\r\n"));
                    if length == 2 { break; }
                    let raw = std::str::from_utf8(&line[..length - 2]).unwrap();
                    if fields == 1 { assert_eq!(raw, "POST /ocr HTTP/1.1"); continue; }
                    let (key, value) = raw.split_once(':').expect("field");
                    let value = value.trim();
                    assert!(!key.eq_ignore_ascii_case("baggage"), "baggage never reaches the OCR wire");
                    if key.eq_ignore_ascii_case("traceparent") {
                        assert_eq!(parent_length, 0, "one parent only");
                        parent_length = value.len();
                        parent[..parent_length].copy_from_slice(value.as_bytes());
                    } else if key.eq_ignore_ascii_case("tracestate") {
                        assert_eq!(state_length, 0, "one canonical state");
                        state_length = value.len();
                        state[..state_length].copy_from_slice(value.as_bytes());
                    }
                }
                let mut body = [0; 1];
                reader.read_exact(&mut body).unwrap();
                let index = usize::from(body[0]);
                assert!(!seen[index]);
                seen[index] = true;
                let expected = &CASES[index];
                assert_eq!(std::str::from_utf8(&parent[..parent_length]).unwrap(), expected.expected_parent.unwrap_or(""));
                assert_eq!(std::str::from_utf8(&state[..state_length]).unwrap(), expected.expected_state.unwrap_or(""));
                reader.get_mut().write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK").unwrap();
                count += 1;
            }
            assert!(seen.into_iter().all(|case| case));
        });
        for pair in 0..3 {
            let mut first = spawn(&work, pair * 2, address);
            let mut second = spawn(&work, pair * 2 + 1, address);
            wait(&mut first);
            wait(&mut second);
        }
        server.join().expect("all wire assertions pass");
    });
    for index in 0..CASES.len() {
        assert_eq!(
            fs::read_to_string(work.join(format!("{index}.out.txt"))).unwrap(),
            "OK"
        );
    }
    fs::remove_dir_all(work).expect("owned test cleanup");
}
