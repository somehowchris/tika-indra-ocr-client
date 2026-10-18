use super::{Failure, PARENTS, check, write_new};
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use prost::Message;
use serde::Serialize;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

const MAX_SPANS: usize = 512;

#[derive(Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    GatewayHttp,
    JavaParse,
    JavaOcr,
    NativeHttp,
    NativeInference,
    #[default]
    Other,
}

#[derive(Default, Serialize)]
struct Span {
    trace: [u8; 16],
    id: [u8; 8],
    parent: [u8; 8],
    kind: Kind,
    state: [u8; 32],
    state_used: usize,
    error: bool,
    start_ns: u64,
    end_ns: u64,
}

pub(super) struct Captured {
    spans: [Span; MAX_SPANS],
    used: usize,
    requests: usize,
    bytes: usize,
}

impl Captured {
    fn new() -> Self {
        Self {
            spans: std::array::from_fn(|_| Span::default()),
            used: 0,
            requests: 0,
            bytes: 0,
        }
    }

    pub(super) fn write(&self, destination: &Path) -> Result<(), Failure> {
        let mut file = super::create_new(destination)?;
        serde_json::to_writer_pretty(&mut file, &self.spans[..self.used])?;
        file.write_all(b"\n")?;
        Ok(())
    }

    /// A trace is admitted by actual gateway, Java and native span IDs, not by matching a
    /// trace ID alone. Missing/invalid contexts must produce distinct new roots.
    pub(super) fn verify(&self) -> Result<(), Failure> {
        check(
            self.requests > 0 && self.used > 0,
            "collector captured no trace exports",
        )?;
        let mut fresh = [[0_u8; 16]; 3];
        let mut fresh_used = 0;
        for span in &self.spans[..self.used] {
            check(
                span.id != [0; 8] && span.trace != [0; 16],
                "zero span identity",
            )?;
            if span.kind == Kind::GatewayHttp && span.parent == [0; 8] {
                check(
                    fresh_used < fresh.len(),
                    "unexpected extra gateway root trace",
                )?;
                check(
                    !fresh[..fresh_used].contains(&span.trace),
                    "fresh requests share a trace",
                )?;
                fresh[fresh_used] = span.trace;
                fresh_used += 1;
                self.success_graph(span, b"")?;
            }
        }
        check(
            fresh_used == 3,
            "missing, invalid and duplicate parent cases need three fresh traces",
        )?;
        for (i, parent) in PARENTS.iter().enumerate() {
            let parsed = indra_telemetry::Parent::parse(parent)
                .ok_or(Failure::Gate("invalid fixed parent"))?;
            let mut matches = self.spans[..self.used].iter().filter(|span| {
                span.kind == Kind::GatewayHttp
                    && span.trace == parsed.trace
                    && span.parent == parsed.span
            });
            let root = matches.next().ok_or(Failure::Gate(
                "incoming remote parent missing from gateway root",
            ))?;
            check(
                matches.next().is_none(),
                "duplicate gateway root for a fixed remote parent",
            )?;
            if i == 3 {
                let parse = self.child(root, Kind::JavaParse)?;
                check(parse.error, "Java parser error was not recorded")?;
                let child = self.ocr_descendant(parse)?;
                check(child.error, "Java OCR process error was not recorded")?;
                check(
                    !self.spans[..self.used]
                        .iter()
                        .any(|span| span.trace == root.trace && span.kind == Kind::NativeHttp),
                    "rejected CLI options unexpectedly reached native OCR",
                )?;
            } else {
                let state: &[u8] = match i {
                    0 => b"case=first",
                    1 => b"case=second",
                    _ => b"",
                };
                self.success_graph(root, state)?;
            }
        }
        let first =
            indra_telemetry::Parent::parse(PARENTS[0]).ok_or(Failure::Gate("fixed parent"))?;
        let second =
            indra_telemetry::Parent::parse(PARENTS[1]).ok_or(Failure::Gate("fixed parent"))?;
        let a = self.spans[..self.used]
            .iter()
            .find(|span| span.trace == first.trace && span.parent == first.span)
            .ok_or(Failure::Gate("first concurrent root"))?;
        let b = self.spans[..self.used]
            .iter()
            .find(|span| span.trace == second.trace && span.parent == second.span)
            .ok_or(Failure::Gate("second concurrent root"))?;
        check(
            a.start_ns < b.end_ns && b.start_ns < a.end_ns,
            "actual gateway request intervals did not overlap",
        )?;
        let java_a = self.child(a, Kind::JavaParse)?;
        let java_b = self.child(b, Kind::JavaParse)?;
        check(
            java_a.start_ns < java_b.end_ns && java_b.start_ns < java_a.end_ns,
            "correlated Java parser intervals did not overlap",
        )
    }

    fn child(&self, parent: &Span, kind: Kind) -> Result<&Span, Failure> {
        let mut matches = self.spans[..self.used].iter().filter(|span| {
            span.kind == kind && span.trace == parent.trace && span.parent == parent.id
        });
        let child = matches
            .next()
            .ok_or(Failure::Gate("required exact span-parent edge is missing"))?;
        check(matches.next().is_none(), "duplicate span-parent edge")?;
        Ok(child)
    }

    fn success_graph(&self, root: &Span, state: &[u8]) -> Result<(), Failure> {
        let parse = self.child(root, Kind::JavaParse)?;
        let ocr = self.ocr_descendant(parse)?;
        let http = self.child(ocr, Kind::NativeHttp)?;
        check(
            !root.error && !parse.error && !ocr.error && !http.error,
            "successful OCR trace has error status",
        )?;
        for span in [root, parse, ocr, http] {
            check(
                &span.state[..span.state_used] == state,
                "tracestate changed between actual process hops",
            )?;
        }
        check(
            self.spans[..self.used].iter().any(|candidate| {
                candidate.kind == Kind::NativeInference
                    && candidate.trace == http.trace
                    && self.descendant(candidate, http)
            }),
            "native inference is not a descendant of the forwarded HTTP span",
        )
    }

    fn ocr_descendant(&self, root: &Span) -> Result<&Span, Failure> {
        let mut matches = self.spans[..self.used].iter().filter(|span| {
            span.trace == root.trace && span.kind == Kind::JavaOcr && self.descendant(span, root)
        });
        let ocr = matches.next().ok_or(Failure::Gate(
            "actual OCR process has no Java parse ancestor",
        ))?;
        check(
            matches.next().is_none(),
            "unexpected duplicate OCR process for one two-page TIFF",
        )?;
        Ok(ocr)
    }

    fn descendant<'a>(&'a self, mut candidate: &'a Span, ancestor: &Span) -> bool {
        for _ in 0..64 {
            if candidate.parent == ancestor.id {
                return true;
            }
            let Some(parent) = self.spans[..self.used]
                .iter()
                .find(|span| span.trace == candidate.trace && span.id == candidate.parent)
            else {
                return false;
            };
            candidate = parent;
        }
        false
    }
}

pub(super) fn collect(
    listener: TcpListener,
    stop: &AtomicBool,
    output: &Path,
) -> Result<Captured, Failure> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut captured = Captured::new();
    while !stop.load(Ordering::Acquire) {
        check(Instant::now() < deadline, "collector aggregate deadline")?;
        match listener.accept() {
            Ok((socket, _)) => receive(socket, &mut captured, output, deadline)?,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(2))
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(captured)
}

fn receive(
    mut socket: TcpStream,
    captured: &mut Captured,
    output: &Path,
    aggregate: Instant,
) -> Result<(), Failure> {
    let deadline = aggregate.min(Instant::now() + Duration::from_secs(5));
    socket.set_read_timeout(Some(Duration::from_millis(250)))?;
    socket.set_write_timeout(Some(Duration::from_secs(1)))?;
    let mut bytes = [0_u8; 262_144];
    let mut used = 0;
    let boundary = loop {
        check(
            used < 8192 && Instant::now() < deadline,
            "collector header limit/deadline",
        )?;
        used += read(&mut socket, &mut bytes[used..8192], deadline)?;
        if let Some(at) = bytes[..used]
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
        {
            break at + 4;
        }
    };
    let header = std::str::from_utf8(&bytes[..boundary])
        .map_err(|_| Failure::Gate("non-ASCII collector headers"))?;
    let trace = header.starts_with("POST /v1/traces HTTP/1.1\r\n");
    check(
        trace || header.starts_with("POST /v1/metrics HTTP/1.1\r\n"),
        "unexpected collector path",
    )?;
    let mut lengths = header.lines().filter_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case("content-length")
            .then_some(value.trim())
    });
    let length: usize = lengths
        .next()
        .ok_or(Failure::Gate("collector requires fixed content length"))?
        .parse()
        .map_err(|_| Failure::Gate("invalid collector body length"))?;
    check(lengths.next().is_none(), "duplicate collector body length")?;
    check(
        !header.lines().any(|line| {
            line.split_once(':').is_some_and(|(key, _)| {
                key.eq_ignore_ascii_case("content-encoding")
                    || key.eq_ignore_ascii_case("transfer-encoding")
            })
        }),
        "collector accepts only the configured uncompressed protobuf",
    )?;
    let end = boundary
        .checked_add(length)
        .ok_or(Failure::Gate("collector body overflow"))?;
    check(
        end <= bytes.len() && used <= end,
        "collector fixed body cap",
    )?;
    while used < end {
        used += read(&mut socket, &mut bytes[used..end], deadline)?;
    }
    let body = &bytes[boundary..end];
    for forbidden in [
        b"INVOICE".as_slice(),
        b"SECOND PAGE",
        b"DOCUMENT_SECRET",
        b"exception.message",
        b"exception.stacktrace",
        b"baggage",
    ] {
        check(
            !body.windows(forbidden.len()).any(|part| part == forbidden),
            "private text/exception payload exported",
        )?;
    }
    captured.bytes = captured
        .bytes
        .checked_add(body.len())
        .ok_or(Failure::Gate("collector total overflow"))?;
    check(
        captured.bytes <= 4_194_304 && captured.requests < 64,
        "collector total capture limit",
    )?;
    write_new(
        &output.join(format!(
            "otlp-{:02}-{}.protobuf",
            captured.requests,
            if trace { "traces" } else { "metrics" }
        )),
        body,
    )?;
    captured.requests += 1;
    if trace {
        // The generated protocol decoder owns its repeated fields. Application
        // capture stays fixed-size and admits no arbitrary payload attributes.
        let request = ExportTraceServiceRequest::decode(body)?;
        for resource in &request.resource_spans {
            for scope in &resource.scope_spans {
                for span in &scope.spans {
                    check(captured.used < MAX_SPANS, "collector span cap")?;
                    check(
                        span.trace_id.len() == 16
                            && span.span_id.len() == 8
                            && (span.parent_span_id.len() == 8 || span.parent_span_id.is_empty()),
                        "invalid exported span identity",
                    )?;
                    check(
                        span.trace_state.len() <= 32,
                        "unexpected test tracestate size",
                    )?;
                    let destination = &mut captured.spans[captured.used];
                    destination.trace.copy_from_slice(&span.trace_id);
                    destination.id.copy_from_slice(&span.span_id);
                    if !span.parent_span_id.is_empty() {
                        destination.parent.copy_from_slice(&span.parent_span_id);
                    }
                    destination.state_used = span.trace_state.len();
                    destination.state[..destination.state_used]
                        .copy_from_slice(span.trace_state.as_bytes());
                    destination.error = span.status.as_ref().is_some_and(|status| status.code == 2);
                    destination.start_ns = span.start_time_unix_nano;
                    destination.end_ns = span.end_time_unix_nano;
                    check(
                        destination.start_ns > 0 && destination.end_ns >= destination.start_ns,
                        "invalid exported span interval",
                    )?;
                    destination.kind = match span.name.as_str() {
                        "Tika HTTP parse" => Kind::GatewayHttp,
                        "tika.parse" => Kind::JavaParse,
                        "tika.ocr.process" => Kind::JavaOcr,
                        "POST /ocr" => Kind::NativeHttp,
                        "ocr.recognition" | "ocr.inference" => Kind::NativeInference,
                        _ => Kind::Other,
                    };
                    for earlier in &captured.spans[..captured.used] {
                        check(
                            earlier.trace != span.trace_id.as_slice()
                                || earlier.id != span.span_id.as_slice(),
                            "duplicate exported span identity",
                        )?;
                    }
                    captured.used += 1;
                }
            }
        }
    }
    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-protobuf\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
    Ok(())
}

fn read(
    socket: &mut TcpStream,
    destination: &mut [u8],
    deadline: Instant,
) -> Result<usize, Failure> {
    loop {
        check(Instant::now() < deadline, "collector body deadline")?;
        match socket.read(destination) {
            Ok(0) => return Err(Failure::Gate("truncated collector request")),
            Ok(n) => return Ok(n),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
}
