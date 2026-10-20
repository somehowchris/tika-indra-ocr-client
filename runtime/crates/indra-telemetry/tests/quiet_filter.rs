//! A quiet JSON log filter must not disable trace export: the OpenTelemetry
//! layer has its own INFO floor. The collector only counts export requests.
#![cfg(feature = "otlp")]

use indra_telemetry::Options;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::{Duration, Instant},
};

fn receive(mut socket: TcpStream, traces: &AtomicUsize) {
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    socket
        .set_write_timeout(Some(Duration::from_secs(2)))
        .expect("write timeout");
    let mut data = [0_u8; 262_144];
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
    let (is_traces, length) = {
        let headers = std::str::from_utf8(&data[..boundary]).expect("ASCII headers");
        let length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().expect("length"))
            })
            .unwrap_or(0);
        (headers.starts_with("POST /v1/traces "), length)
    };
    while used < boundary + length {
        let read = socket.read(&mut data[used..]).expect("body bytes");
        assert!(read > 0, "complete body");
        used += read;
    }
    if is_traces {
        traces.fetch_add(1, Ordering::AcqRel);
    }
    socket
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        .expect("acknowledgement");
}

#[test]
fn warn_log_filter_still_exports_info_spans() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("local collector");
    listener.set_nonblocking(true).expect("nonblocking accept");
    let address = listener.local_addr().expect("collector address");
    let stop = AtomicBool::new(false);
    let traces = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
                match listener.accept() {
                    Ok((socket, _)) => receive(socket, &traces),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("collector accept: {error}"),
                }
            }
        });
        let options = Options {
            otlp_endpoint: Some(format!("http://{address}")),
            log_filter: String::from("warn"),
            trace_sample_ratio: 1.0,
            export_interval_ms: 60_000,
            export_timeout_ms: 2000,
            ..Options::default()
        };
        let guard = indra_telemetry::initialize(&options).expect("real SDK setup");
        let span = indra_telemetry::request_span("0123456789abcdef0123456789abcdef", None, None);
        span.in_scope(|| {
            tracing::info!(
                event = "quiet_filter_probe",
                "below the log filter, inside the span"
            );
        });
        drop(span);
        guard.shutdown().expect("bounded flush");
        stop.store(true, Ordering::Release);
    });
    assert!(
        traces.load(Ordering::Acquire) >= 1,
        "a warn-level log filter must not stop span export"
    );
}
