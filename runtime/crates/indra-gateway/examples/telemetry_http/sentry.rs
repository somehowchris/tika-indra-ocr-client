//! Bounded actual Sentry envelope sink. No vendor endpoint or synthetic SDK events.
use super::{Failure, check, write_new};
use serde::Serialize;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
#[derive(Default, Serialize)]
pub(super) struct Captured {
    pub envelopes: usize,
    pub bytes: usize,
    pub gateway_events: usize,
}
pub(super) fn collect(
    listener: TcpListener,
    stop: &AtomicBool,
    output: &Path,
) -> Result<Captured, Failure> {
    listener.set_nonblocking(true)?;
    let due = Instant::now() + Duration::from_secs(300);
    let mut captured = Captured::default();
    while !stop.load(Ordering::Acquire) {
        check(Instant::now() < due, "Sentry receiver aggregate deadline")?;
        match listener.accept() {
            Ok((socket, peer)) => {
                check(peer.ip().is_loopback(), "non-loopback Sentry peer")?;
                receive(socket, output, &mut captured, due)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(2))
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(captured)
}
fn receive(
    mut socket: TcpStream,
    output: &Path,
    captured: &mut Captured,
    aggregate: Instant,
) -> Result<(), Failure> {
    let due = aggregate.min(Instant::now() + Duration::from_secs(5));
    socket.set_read_timeout(Some(Duration::from_millis(100)))?;
    socket.set_write_timeout(Some(Duration::from_secs(1)))?;
    let mut bytes = [0_u8; 73728];
    let mut used = 0;
    let boundary = loop {
        check(
            used < 8192 && Instant::now() < due,
            "Sentry header limit/deadline",
        )?;
        used += read(&mut socket, &mut bytes[used..8192], due)?;
        if let Some(at) = bytes[..used].windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
    };
    super::audit::validate_export_headers(&bytes[..boundary])?;
    let header = std::str::from_utf8(&bytes[..boundary])
        .map_err(|_| Failure::Gate("Sentry header UTF-8"))?;
    check(
        header.starts_with("POST /api/1/envelope/ HTTP/1.1\r\n"),
        "unexpected Sentry route",
    )?;
    let mut lengths = header.lines().filter_map(|line| {
        let (k, v) = line.split_once(':')?;
        k.eq_ignore_ascii_case("content-length").then_some(v.trim())
    });
    let length = lengths
        .next()
        .ok_or(Failure::Gate("Sentry requires content length"))?
        .parse::<usize>()
        .map_err(|_| Failure::Gate("invalid Sentry content length"))?;
    check(
        lengths.next().is_none() && length <= 65536,
        "duplicate/oversize Sentry length",
    )?;
    check(
        !header.lines().any(|line| {
            line.split_once(':').is_some_and(|(key, _)| {
                key.eq_ignore_ascii_case("transfer-encoding")
                    || key.eq_ignore_ascii_case("content-encoding")
            })
        }),
        "Sentry compression/chunking forbidden",
    )?;
    let end = boundary
        .checked_add(length)
        .ok_or(Failure::Gate("Sentry body overflow"))?;
    check(
        end <= bytes.len() && used <= end,
        "Sentry body cap/pipelining",
    )?;
    while used < end {
        used += read(&mut socket, &mut bytes[used..end], due)?;
    }
    let body = &bytes[boundary..end];
    check(
        captured.envelopes < 8 && captured.bytes + length <= 262144,
        "Sentry aggregate capture cap",
    )?;
    write_new(
        &output.join(format!("sentry-{:02}.envelope", captured.envelopes)),
        body,
    )?;
    // Keep the bounded wire evidence even when domain or privacy admission fails.
    // The enclosing evidence directory is private; rejected payloads are never
    // accepted, acknowledged, or included in a successful study.
    verify_envelope(body)?;
    captured.envelopes += 1;
    captured.bytes += length;
    captured.gateway_events += 1;
    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
    Ok(())
}
fn read(socket: &mut TcpStream, destination: &mut [u8], due: Instant) -> Result<usize, Failure> {
    loop {
        check(Instant::now() < due, "Sentry body deadline")?;
        match socket.read(destination) {
            Ok(0) => return Err(Failure::Gate("truncated Sentry request")),
            Ok(n) => return Ok(n),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(e.into()),
        }
    }
}
pub(super) fn verify_envelope(body: &[u8]) -> Result<(), Failure> {
    for forbidden in [
        b"DOCUMENT_SECRET".as_slice(),
        b"SDK_PRIVATE_CANARY",
        b"COOKIE_PRIVATE_CANARY",
        b"INVOICE",
        b"SECOND PAGE",
        b"exception.message",
        b"exception.stacktrace",
        b"baggage",
    ] {
        check(
            !body.windows(forbidden.len()).any(|s| s == forbidden),
            "private Sentry payload",
        )?;
    }
    let first = body
        .iter()
        .position(|b| *b == b'\n')
        .ok_or(Failure::Gate("missing Sentry envelope header"))?;
    check(first <= 8192, "Sentry envelope header cap")?;
    let _: serde_json::Value = serde_json::from_slice(&body[..first])?;
    let rest = &body[first + 1..];
    let second = rest
        .iter()
        .position(|b| *b == b'\n')
        .ok_or(Failure::Gate("missing Sentry item header"))?;
    check(second <= 1024, "Sentry item header cap")?;
    let item: serde_json::Value = serde_json::from_slice(&rest[..second])?;
    check(item["type"] == "event", "unexpected Sentry item type")?;
    let payload = &rest[second + 1..];
    let data = payload.strip_suffix(b"\n").unwrap_or(payload);
    if let Some(length) = item.get("length") {
        check(
            length.as_u64() == Some(data.len() as u64),
            "Sentry item length differs",
        )?;
    }
    let event: serde_json::Value = serde_json::from_slice(data)?;
    let fields = event
        .as_object()
        .ok_or(Failure::Gate("Sentry event is not an object"))?;
    check(
        fields.keys().all(|name| {
            matches!(
                name.as_str(),
                "event_id"
                    | "level"
                    | "version"
                    | "type"
                    | "logger"
                    | "platform"
                    | "timestamp"
                    | "release"
                    | "environment"
                    | "message"
                    | "exception"
                    | "tags"
                    | "extra"
                    | "breadcrumbs"
                    | "sdk"
                    | "contexts"
            )
        }),
        "unexpected Sentry event payload field",
    )?;
    check(
        // sentry-types0.49.3 Event::level defaults to Error and skips that
        // default during serialization; an explicit null or other level fails.
        event.get("level").is_none_or(|level| level == "error")
            && event["logger"] == "indra.runtime"
            && event["tags"]["failure.class"] == "java_exited"
            && event["tags"]["ocr.phase"] == "gateway",
        "Sentry event not actual gateway Java failure",
    )?;
    check(
        event["message"]
            == "Service operation failed; inspect the correlated trace and finite failure class",
        "unexpected Sentry message",
    )?;
    check(
        event["exception"]["values"]
            .as_array()
            .is_some_and(|v| v.len() == 1)
            && event["exception"]["values"][0]["type"] == "java_exited"
            && event["exception"]["values"][0]["value"] == "A bounded service operation failed",
        "unexpected Sentry exception",
    )?;
    check(
        event.get("request").is_none()
            && event.get("user").is_none()
            && event.get("server_name").is_none(),
        "private Sentry request/user/server data",
    )?;
    check(
        event["exception"]["values"][0]
            .as_object()
            .is_some_and(|exception| {
                exception
                    .keys()
                    .all(|key| matches!(key.as_str(), "type" | "value"))
            }),
        "Sentry stack payload forbidden",
    )?;
    let tags = event["tags"]
        .as_object()
        .ok_or(Failure::Gate("Sentry tags absent"))?;
    check(
        tags.iter().all(|(key, value)| match key.as_str() {
            "failure.class" => value == "java_exited",
            "ocr.phase" => value == "gateway",
            "request_id" | "trace_id" => value
                .as_str()
                .is_some_and(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit())),
            "span_id" => value
                .as_str()
                .is_some_and(|s| s.len() == 16 && s.bytes().all(|b| b.is_ascii_hexdigit())),
            _ => false,
        }),
        "unbounded Sentry tags",
    )?;
    let extra = event["extra"]
        .as_object()
        .ok_or(Failure::Gate("Sentry timing context absent"))?;
    check(
        extra.len() == 1
            && extra
                .get("elapsed_ms")
                .and_then(serde_json::Value::as_f64)
                .is_some_and(|v| v.is_finite() && v >= 0.0),
        "unbounded Sentry extra context",
    )?;
    check(
        event.get("contexts").is_none_or(|value| {
            value.as_object().is_some_and(|contexts| {
                contexts.is_empty()
                    || (contexts.len() == 1
                        && contexts.get("trace").is_some_and(|value| {
                            value.as_object().is_some_and(|trace| {
                                trace.len() == 3
                                    && trace.get("type").is_some_and(|value| value == "trace")
                                    && wire_id(trace.get("trace_id"), 32)
                                    && wire_id(trace.get("span_id"), 16)
                            })
                        }))
            })
        }),
        "unexpected Sentry context payload",
    )?;
    let breadcrumbs = event["breadcrumbs"]["values"]
        .as_array()
        .ok_or(Failure::Gate("Sentry lifecycle breadcrumbs absent"))?;
    check(
        breadcrumbs.len() == 2
            && breadcrumbs[0]["category"] == "service.lifecycle"
            && breadcrumbs[0]["message"]
                == "Tika admission stopped before terminating and reaping its process group"
            && breadcrumbs[1]["category"] == "service.failure"
            && breadcrumbs[1]["message"] == "java_exited"
            && breadcrumbs.iter().all(|crumb| {
                crumb
                    .get("data")
                    .is_none_or(|data| data.as_object().is_some_and(|m| m.is_empty()))
            }),
        "unbounded Sentry breadcrumbs",
    )?;
    check(
        breadcrumbs.iter().all(|crumb| {
            crumb.as_object().is_some_and(|fields| {
                fields.keys().all(|key| {
                    matches!(
                        key.as_str(),
                        "category" | "message" | "level" | "timestamp" | "type" | "data"
                    )
                })
            })
        }),
        "unexpected Sentry breadcrumb payload field",
    )?;
    Ok(())
}

// sentry-core0.49.3 Scope::apply_propagation_context inserts only this SDK
// identity when no transaction is active. It is not proof of an OTLP parent edge.
fn wire_id(value: Option<&serde_json::Value>, length: usize) -> bool {
    value
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| {
            value.len() == length
                && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                && value.bytes().any(|byte| byte != b'0')
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    static EVENT: &str = r#"{"level":"error","logger":"indra.runtime","message":"Service operation failed; inspect the correlated trace and finite failure class","extra":{"elapsed_ms":1.0},"breadcrumbs":{"values":[{"category":"service.lifecycle","message":"Tika admission stopped before terminating and reaping its process group"},{"category":"service.failure","message":"java_exited"}]},"tags":{"failure.class":"java_exited","ocr.phase":"gateway"},"exception":{"values":[{"type":"java_exited","value":"A bounded service operation failed"}]}}"#;
    fn envelope(event: &str) -> String {
        format!(
            "{{}}\n{{\"type\":\"event\",\"length\":{}}}\n{event}\n",
            event.len()
        )
    }
    #[test]
    fn actual_pinned_sdk_envelope_has_default_error_and_only_finite_trace_identity() {
        static ACTUAL: &[u8] = include_bytes!("fixtures/sentry-java-exited-0.49.3.envelope");
        verify_envelope(ACTUAL).expect("actual classified gateway failure wire envelope");
        let raw = std::str::from_utf8(ACTUAL).unwrap();
        let data = raw.splitn(3, '\n').nth(2).unwrap().trim_end();
        let event: serde_json::Value = serde_json::from_str(data).unwrap();
        assert!(event.get("level").is_none());
        for (pointer, value) in [
            ("/level", serde_json::Value::Null),
            ("/level", serde_json::json!("warning")),
            (
                "/tags/failure.class",
                serde_json::json!("unrelated_failure"),
            ),
            (
                "/contexts/trace/trace_id",
                serde_json::json!("00000000000000000000000000000000"),
            ),
            (
                "/contexts/trace/span_id",
                serde_json::json!("not-a-valid-id"),
            ),
            (
                "/contexts/trace/type",
                serde_json::json!("arbitrary_context"),
            ),
            (
                "/contexts/trace/document",
                serde_json::json!("not an allowed field"),
            ),
            (
                "/contexts/document",
                serde_json::json!({"text":"SDK_PRIVATE_CANARY"}),
            ),
            (
                "/request",
                serde_json::json!({"headers":{"Cookie":"COOKIE_PRIVATE_CANARY"}}),
            ),
        ] {
            let mut changed: serde_json::Value = serde_json::from_str(data).unwrap();
            let (parent, key) = pointer.rsplit_once('/').unwrap();
            changed
                .pointer_mut(parent)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert(String::from(key), value);
            let json = serde_json::to_string(&changed).unwrap();
            assert!(
                verify_envelope(envelope(&json).as_bytes()).is_err(),
                "{pointer}"
            );
        }
    }
    #[test]
    fn rejected_wire_event_is_retained_without_acknowledgment_or_acceptance() {
        use std::{net::Shutdown, os::unix::fs::DirBuilderExt};
        let path =
            std::env::temp_dir().join(format!("indra-sentry-rejection-{}", std::process::id()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let body = envelope(&EVENT.replace("java_exited", "unclassified_failure"));
        let mut captured = Captured::default();
        let (result, response) = std::thread::scope(|scope| {
            let receiver = scope.spawn(|| {
                let (socket, _) = listener.accept().unwrap();
                receive(
                    socket,
                    &path,
                    &mut captured,
                    Instant::now() + Duration::from_secs(5),
                )
            });
            let mut client = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            write!(client,
                "POST /api/1/envelope/ HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ).unwrap();
            client.shutdown(Shutdown::Write).unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            (receiver.join().unwrap(), response)
        });
        let retained = std::fs::read(path.join("sentry-00.envelope")).unwrap();
        std::fs::remove_dir_all(path).unwrap();
        assert!(result.is_err());
        assert!(response.is_empty());
        assert_eq!(captured.envelopes, 0);
        assert_eq!(captured.gateway_events, 0);
        assert_eq!(retained, body.as_bytes());
    }
    #[test]
    fn domain_event_accepted_but_private_or_ambiguous_payload_rejected() {
        verify_envelope(envelope(EVENT).as_bytes()).expect("bounded classified event fixture");
        for bad in [
            EVENT.replace("java_exited", "arbitrary_document_failure"),
            EVENT.replace(
                "\"elapsed_ms\":1.0",
                "\"elapsed_ms\":1.0,\"document\":\"unseen mathematical response\"",
            ),
            EVENT.replace(
                "\"ocr.phase\":\"gateway\"",
                "\"ocr.phase\":\"gateway\",\"document\":\"unseen text\"",
            ),
            EVENT.replace("bounded service operation", "SDK_PRIVATE_CANARY"),
            EVENT.replace("\"level\":\"error\"", "\"request\":{},\"level\":\"error\""),
            EVENT.replace(
                "\"value\":\"A bounded service operation failed\"",
                "\"stacktrace\":{},\"value\":\"A bounded service operation failed\"",
            ),
        ] {
            assert!(verify_envelope(envelope(&bad).as_bytes()).is_err());
        }
        assert!(verify_envelope(b"{}\n{\"type\":\"event\",\"length\":1}\n{}\n").is_err());
        assert!(
            verify_envelope(format!("{}{}", envelope(EVENT), envelope(EVENT)).as_bytes()).is_err()
        );
    }
}
