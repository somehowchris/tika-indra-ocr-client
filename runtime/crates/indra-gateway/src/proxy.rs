//! One bounded task owns each admitted upstream exchange. Full upload validation
//! precedes Java, allowing malformed/disconnected uploads to release capacity
//! without abandoning a started parse. Responses stream through one 16 KiB
//! channel slot; disconnected clients become a sink until the backend reaches EOF.
use crate::{
    configuration::MAX_HEADERS,
    error::ProxyError,
    service::{self, Frontend, Shared, Upload},
};
use axum::{
    body::Body,
    extract::State,
    http::{self, HeaderMap, HeaderValue, Method, Request, StatusCode, header},
    response::Response,
};
use bytes::Bytes;
use http_body::{Body as HttpBody, Frame};
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use std::{
    net::SocketAddr,
    pin::Pin,
    sync::atomic::Ordering,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    net::TcpStream,
    sync::{mpsc, oneshot},
    time::Instant,
};
use tracing::Instrument;

const STREAM_CHUNK: usize = 16 * 1024;
enum StreamItem {
    Chunk(Bytes),
    Complete,
}
struct Head {
    status: StatusCode,
    headers: HeaderMap,
    receiver: mpsc::Receiver<StreamItem>,
}

struct StreamBody {
    receiver: mpsc::Receiver<StreamItem>,
    pending: Option<Bytes>,
    finished: bool,
    front: Option<Frontend>,
    outcome: &'static str,
}
struct FinalChunk {
    bytes: Bytes,
    _front: Frontend,
}
impl AsRef<[u8]> for FinalChunk {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
impl HttpBody for StreamBody {
    type Data = Bytes;
    type Error = ProxyError;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, ProxyError>>> {
        let this = self.get_mut();
        if this.finished {
            return Poll::Ready(None);
        }
        loop {
            match this.receiver.poll_recv(cx) {
                Poll::Ready(Some(StreamItem::Chunk(bytes))) => {
                    if let Some(previous) = this.pending.replace(bytes) {
                        return Poll::Ready(Some(Ok(Frame::data(previous))));
                    }
                }
                Poll::Ready(Some(StreamItem::Complete)) => {
                    this.finished = true;
                    if let Some(front) = &mut this.front {
                        front.record(this.outcome);
                    }
                    return Poll::Ready(this.pending.take().map(|bytes| {
                        Ok(Frame::data(Bytes::from_owner(FinalChunk {
                            bytes,
                            _front: this.front.take().expect("response owner"),
                        })))
                    }));
                }
                Poll::Ready(None) => {
                    this.finished = true;
                    if let Some(front) = &mut this.front {
                        front.record("response_incomplete");
                    }
                    return Poll::Ready(Some(Err(ProxyError::Upstream)));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

pub(crate) struct RawDocument;
impl utoipa::PartialSchema for RawDocument {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::Schema> {
        use utoipa::openapi::schema::{KnownFormat, ObjectBuilder, SchemaFormat, Type};
        ObjectBuilder::new()
            .schema_type(Type::String)
            .format(Some(SchemaFormat::KnownFormat(KnownFormat::Binary)))
            .into()
    }
}
impl utoipa::ToSchema for RawDocument {}

#[utoipa::path(put, path="/tika", tag="Tika",
    request_body(description="Raw document bytes. The gateway validates and stages the bounded upload before starting Java. Other enabled Tika paths and methods use the same admission and deadline policy.", content((RawDocument="application/octet-stream"), (RawDocument="application/pdf"), (RawDocument="image/png"))),
    params(("Content-Length"=usize, Header, description="Exact decimal length; default maximum 64 MiB. Chunked uploads and trailers are rejected."), ("traceparent"=Option<String>, Header, description="Optional validated W3C parent for the gateway trace.")),
    responses(
        (status=200, description="Tika response streamed unchanged, normally UTF-8 text; Accept controls Tika's response format", body=String, content_type="text/plain"),
        (status=400, description="Unsupported or invalid HTTP headers/body", body=service::ApiError),
        (status=411, description="Missing or invalid single Content-Length", body=service::ApiError),
        (status=413, description="Upload exceeds configured limit", body=service::ApiError),
        (status=502, description="Tika upstream/protocol/response bound failed; a failure after headers aborts the response stream", body=service::ApiError),
        (status=503, description="Starting, saturated, restarting, draining or failed; Retry-After 1", body=service::ApiError),
        (status=504, description="Absolute upload/request deadline expired", body=service::ApiError)
    )
)]
pub(crate) async fn handle(
    State(shared): State<&'static Shared>,
    request: Request<Body>,
) -> Response {
    let (mut front, backend, mut upload) = match shared.admit() {
        Some(owners) => owners,
        None => {
            indra_telemetry::gateway_request_finished("unavailable", Duration::ZERO);
            let mut response = service::api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "Gateway is starting, saturated, restarting, draining or failed",
            );
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
            return response;
        }
    };
    let started = Instant::now();
    let due = started + Duration::from_secs(shared.settings.request_timeout_seconds);
    let id = shared.next_request.fetch_add(1, Ordering::Relaxed);
    let request_id = format!("{id:016x}");
    let mut trace = indra_telemetry::Carrier::from_headers(
        request
            .headers()
            .get_all("traceparent")
            .iter()
            .map(|h| h.to_str().ok()),
        request
            .headers()
            .get_all("tracestate")
            .iter()
            .map(|h| h.to_str().ok()),
    );
    let span = indra_telemetry::http_span(
        method_label(request.method()),
        "/tika/{path}",
        "Tika HTTP parse",
        &request_id,
        trace.parent(),
        trace.state(),
    );
    let response = async {
        let (mut parts, mut body) = request.into_parts();
        let length = match request_headers(
            &parts.headers,
            &parts.method,
            shared.settings.max_request_bytes,
        ) {
            Ok(length) => length,
            Err(error) => return rejected(error, &mut front),
        };
        if parts.uri.scheme().is_some()
            || parts.uri.authority().is_some()
            || parts.uri.path().len() > 2048
            || parts.uri.query().is_some_and(|q| q.len() > 4096)
        {
            return rejected(ProxyError::Headers, &mut front);
        }
        let upload_due =
            due.min(started + Duration::from_secs(shared.settings.upload_timeout_seconds));
        match tokio::time::timeout_at(upload_due, receive_upload(&mut body, length, &mut upload))
            .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return rejected(error, &mut front),
            Err(_) => return rejected(ProxyError::Deadline, &mut front),
        }
        if let Err(error) = strip_hop_headers(&mut parts.headers) {
            return rejected(error, &mut front);
        }
        parts.headers.insert(
            header::HOST,
            HeaderValue::from_str(&shared.settings.backend.to_string())
                .expect("numeric socket address"),
        );
        parts
            .headers
            .insert(header::CONNECTION, HeaderValue::from_static("close"));
        // This future is polled inside the gateway request span. Forward that
        // current span as Tika's parent, never merely the original client ID.
        forward_trace(&mut parts.headers, &mut trace);
        // The pooled input remains owned until Hyper's final Bytes reference
        // drops, independently of both the response stream and backend task.
        let is_head = parts.method == Method::HEAD;
        let request = Request::from_parts(parts, Full::new(Bytes::from_owner(upload)));
        let (head_tx, head_rx) = oneshot::channel();
        // Admission caps these tasks; detachment preserves ownership when the
        // frontend handler is dropped. No task exists for rejected requests.
        tokio::spawn(
            async move {
                let outcome = exchange(shared, request, head_tx, due).await;
                match outcome {
                    Ok(()) => {
                        indra_telemetry::gateway_backend_finished("complete", started.elapsed())
                    }
                    Err(error) => {
                        indra_telemetry::gateway_backend_finished(error.code(), started.elapsed());
                        tracing::error!(event="tika_exchange_failed", error.type=error.code());
                        shared.report(error.code(), Some(&format!("{id:016x}")), started.elapsed());
                        // After ambiguous EOF/timeout we cannot prove Java stopped
                        // parsing. Keep its lease until the supervisor kills/reaps
                        // the process group; no false recovery frees hidden work.
                        shared.fail(error.code());
                        shared.stopped().await;
                    }
                }
                drop(backend);
            }
            .in_current_span(),
        );
        match tokio::time::timeout_at(due, head_rx).await {
            Ok(Ok(Ok(head))) => {
                let outcome = if head.status.is_success() {
                    "complete"
                } else {
                    "tika_status_error"
                };
                if is_head
                    || matches!(
                        head.status,
                        StatusCode::NO_CONTENT | StatusCode::NOT_MODIFIED
                    )
                {
                    front.record(outcome);
                }
                tracing::Span::current().record("http.response.status_code", head.status.as_u16());
                let mut response = Response::new(Body::new(StreamBody {
                    receiver: head.receiver,
                    pending: None,
                    finished: false,
                    front: Some(front),
                    outcome,
                }));
                *response.status_mut() = head.status;
                *response.headers_mut() = head.headers;
                service::close(response)
            }
            Ok(Ok(Err(error))) => rejected(error, &mut front),
            Ok(Err(_)) => rejected(ProxyError::Upstream, &mut front),
            Err(_) => rejected(ProxyError::Deadline, &mut front),
        }
    }
    .instrument(span)
    .await;
    let mut response = response;
    response.headers_mut().insert(
        "x-request-id",
        HeaderValue::from_str(&request_id).expect("hex request ID"),
    );
    response
}

fn forward_trace(headers: &mut HeaderMap, trace: &mut indra_telemetry::Carrier) {
    indra_telemetry::inject_current(trace);
    headers.remove("traceparent");
    headers.remove("tracestate");
    headers.remove("baggage");
    // HeaderMap owns each inserted HeaderValue. These bounded library header
    // copies do not clone document buffers or allocate new application Strings.
    if let Some(parent) = trace.parent() {
        headers.insert(
            "traceparent",
            HeaderValue::from_str(parent).expect("validated ASCII parent"),
        );
    }
    if let Some(state) = trace.state() {
        headers.insert(
            "tracestate",
            HeaderValue::from_str(state).expect("validated ASCII state"),
        );
    }
}

fn method_label(method: &Method) -> &'static str {
    match *method {
        Method::GET => "GET",
        Method::HEAD => "HEAD",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::DELETE => "DELETE",
        Method::OPTIONS => "OPTIONS",
        _ => "OTHER",
    }
}

fn request_headers(
    headers: &HeaderMap,
    method: &Method,
    maximum: usize,
) -> Result<usize, ProxyError> {
    if header_size(headers) > MAX_HEADERS
        || headers.contains_key(header::TRANSFER_ENCODING)
        || headers.contains_key(header::UPGRADE)
        || headers.contains_key(header::TRAILER)
    {
        return Err(ProxyError::Headers);
    }
    let mut lengths = headers.get_all(header::CONTENT_LENGTH).iter();
    let length = lengths.next();
    if lengths.next().is_some() {
        return Err(ProxyError::Length);
    }
    let length = match length {
        Some(value) => {
            let value = value.to_str().map_err(|_| ProxyError::Length)?;
            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(ProxyError::Length);
            }
            value.parse::<usize>().map_err(|_| ProxyError::Length)?
        }
        None if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) => 0,
        None => return Err(ProxyError::Length),
    };
    if length > maximum {
        return Err(ProxyError::TooLarge);
    }
    Ok(length)
}

async fn receive_upload(
    body: &mut Body,
    length: usize,
    upload: &mut Upload,
) -> Result<(), ProxyError> {
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| ProxyError::Upload)?;
        let bytes = frame.into_data().map_err(|_| ProxyError::Upload)?;
        if bytes.len() > length - upload.bytes.len() {
            return Err(ProxyError::Upload);
        }
        upload.bytes.extend_from_slice(&bytes);
    }
    if upload.bytes.len() == length {
        Ok(())
    } else {
        Err(ProxyError::Upload)
    }
}

fn header_size(headers: &HeaderMap) -> usize {
    headers
        .iter()
        .map(|(key, value)| key.as_str().len() + value.as_bytes().len() + 4)
        .sum()
}

fn strip_hop_headers(headers: &mut HeaderMap) -> Result<(), ProxyError> {
    // Remove all Connection-nominated fields before standard hop-by-hop fields.
    // Repeated Connection lines are legal; fixed storage bounds their tokens.
    let mut names: [Option<http::HeaderName>; 64] = std::array::from_fn(|_| None);
    let mut count = 0;
    for value in headers.get_all(header::CONNECTION) {
        for name in value.to_str().map_err(|_| ProxyError::Headers)?.split(',') {
            if count == names.len() {
                return Err(ProxyError::Headers);
            }
            let name = name.trim();
            if name.is_empty() {
                return Err(ProxyError::Headers);
            }
            let parsed =
                http::HeaderName::from_bytes(name.as_bytes()).map_err(|_| ProxyError::Headers)?;
            // Hop-by-hop nomination cannot erase framing or route semantics.
            if matches!(parsed.as_str(), "content-length" | "host") {
                return Err(ProxyError::Headers);
            }
            names[count] = Some(parsed);
            count += 1;
        }
    }
    for name in names.into_iter().flatten() {
        headers.remove(name);
    }
    for name in [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "proxy-connection",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
        "expect",
    ] {
        headers.remove(name);
    }
    Ok(())
}

async fn exchange(
    shared: &'static Shared,
    request: Request<Full<Bytes>>,
    head: oneshot::Sender<Result<Head, ProxyError>>,
    due: Instant,
) -> Result<(), ProxyError> {
    let result = tokio::time::timeout_at(due, async {
        let socket = TcpStream::connect(shared.settings.backend)
            .await
            .map_err(|_| ProxyError::Upstream)?;
        socket.set_nodelay(true).map_err(|_| ProxyError::Upstream)?;
        let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
            .max_buf_size(STREAM_CHUNK)
            .handshake(TokioIo::new(socket))
            .await
            .map_err(|_| ProxyError::Upstream)?;
        let forward = async move {
            let response = sender.send_request(request).await;
            drop(sender);
            let mut response = match response {
                Ok(response) => response,
                Err(_) => {
                    let _ = head.send(Err(ProxyError::Upstream));
                    return Err(ProxyError::Upstream);
                }
            };
            if header_size(response.headers()) > MAX_HEADERS
                || response.status() == StatusCode::SWITCHING_PROTOCOLS
            {
                let _ = head.send(Err(ProxyError::Upstream));
                return Err(ProxyError::Upstream);
            }
            if response
                .body()
                .size_hint()
                .upper()
                .is_some_and(|n| n > shared.settings.max_response_bytes as u64)
            {
                let _ = head.send(Err(ProxyError::ResponseLimit));
                return Err(ProxyError::ResponseLimit);
            }
            let mut headers = std::mem::take(response.headers_mut());
            if strip_hop_headers(&mut headers).is_err() {
                let _ = head.send(Err(ProxyError::Upstream));
                return Err(ProxyError::Upstream);
            }
            let (tx, receiver) = mpsc::channel(1);
            let mut output = if head
                .send(Ok(Head {
                    status: response.status(),
                    headers,
                    receiver,
                }))
                .is_ok()
            {
                Some(tx)
            } else {
                None
            };
            let mut bytes_seen = 0_usize;
            while let Some(frame) = response.body_mut().frame().await {
                let frame = frame.map_err(|_| ProxyError::Upstream)?;
                let mut bytes = match frame.into_data() {
                    Ok(bytes) => bytes,
                    Err(_) => return Err(ProxyError::Upstream),
                };
                bytes_seen = bytes_seen
                    .checked_add(bytes.len())
                    .ok_or(ProxyError::ResponseLimit)?;
                if bytes_seen > shared.settings.max_response_bytes {
                    return Err(ProxyError::ResponseLimit);
                }
                while !bytes.is_empty() {
                    let chunk = bytes.split_to(bytes.len().min(STREAM_CHUNK));
                    if let Some(sender) = &output {
                        let stall = due.min(
                            Instant::now()
                                + Duration::from_secs(shared.settings.client_stall_seconds),
                        );
                        if !matches!(
                            tokio::time::timeout_at(stall, sender.send(StreamItem::Chunk(chunk)))
                                .await,
                            Ok(Ok(()))
                        ) {
                            // Retain native work, but stop backpressure from a
                            // disconnected or stalled public response reader.
                            tracing::info!(event = "frontend_detached", bytes = bytes_seen);
                            output = None;
                        }
                    }
                }
            }
            if let Some(sender) = output {
                let stall = due.min(
                    Instant::now() + Duration::from_secs(shared.settings.client_stall_seconds),
                );
                let _ = tokio::time::timeout_at(stall, sender.send(StreamItem::Complete)).await;
            }
            Ok(())
        };
        let (_, result) = tokio::join!(connection, forward);
        result
    })
    .await;
    match result {
        Ok(result) => result,
        Err(_) => Err(ProxyError::Deadline),
    }
}

fn error_response(error: ProxyError) -> Response {
    let status = match error {
        ProxyError::Headers | ProxyError::Upload => StatusCode::BAD_REQUEST,
        ProxyError::Length => StatusCode::LENGTH_REQUIRED,
        ProxyError::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
        ProxyError::Deadline => StatusCode::GATEWAY_TIMEOUT,
        _ => StatusCode::BAD_GATEWAY,
    };
    service::api_error(
        status,
        error.code(),
        match error {
            ProxyError::Headers => "Invalid or unsupported HTTP headers",
            ProxyError::Length => {
                "Provide one decimal Content-Length; chunked uploads are not accepted"
            }
            ProxyError::Upload => "Upload is truncated, oversized, invalid, or has trailers",
            ProxyError::TooLarge => "Upload exceeds the configured bound",
            ProxyError::Deadline => "The absolute request deadline expired",
            _ => "Tika did not complete a bounded HTTP response",
        },
    )
}

fn rejected(error: ProxyError, front: &mut Frontend) -> Response {
    front.record(error.code());
    tracing::Span::current().record("error.type", error.code());
    let response = error_response(error);
    tracing::Span::current().record("http.response.status_code", response.status().as_u16());
    response
}

/// Distinguishes a wrong successful version from a transient fork startup gap.
pub(crate) async fn version(address: SocketAddr) -> Result<bool, ProxyError> {
    dependency_probe(address, Dependency::TikaVersion).await
}

pub(crate) async fn ocr_ready(address: SocketAddr) -> Result<bool, ProxyError> {
    dependency_probe(address, Dependency::OcrReadiness).await
}

enum Dependency {
    TikaVersion,
    OcrReadiness,
}

async fn dependency_probe(address: SocketAddr, dependency: Dependency) -> Result<bool, ProxyError> {
    tokio::time::timeout(Duration::from_millis(500), async {
        let socket = TcpStream::connect(address)
            .await
            .map_err(|_| ProxyError::Upstream)?;
        let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
            .max_buf_size(STREAM_CHUNK)
            .handshake(TokioIo::new(socket))
            .await
            .map_err(|_| ProxyError::Upstream)?;
        let probe = async move {
            let request = Request::builder()
                .uri(match dependency {
                    Dependency::TikaVersion => "/version",
                    Dependency::OcrReadiness => "/readyz",
                })
                .header(header::HOST, "localhost")
                .header(header::CONNECTION, "close")
                .body(Full::new(Bytes::new()))
                .expect("fixed probe request");
            let mut response = sender
                .send_request(request)
                .await
                .map_err(|_| ProxyError::Upstream)?;
            drop(sender);
            if response.status() != StatusCode::OK {
                return Err(ProxyError::Upstream);
            }
            let mut content = [0; 1024];
            let mut used = 0;
            while let Some(frame) = response.body_mut().frame().await {
                let frame = frame
                    .map_err(|_| ProxyError::Upstream)?
                    .into_data()
                    .map_err(|_| ProxyError::Upstream)?;
                if frame.len() > content.len() - used {
                    return Ok(false);
                }
                content[used..used + frame.len()].copy_from_slice(&frame);
                used += frame.len();
            }
            match dependency {
                Dependency::TikaVersion => Ok(std::str::from_utf8(&content[..used])
                    .is_ok_and(|s| s.trim() == "Apache Tika 3.3.2")),
                Dependency::OcrReadiness => {
                    #[derive(serde::Deserialize)]
                    struct Ready<'a> {
                        phase: &'a str,
                    }
                    Ok(serde_json::from_slice::<Ready<'_>>(&content[..used])
                        .is_ok_and(|status| status.phase == "ready"))
                }
            }
        };
        let (_, result) = tokio::join!(connection, probe);
        result
    })
    .await
    .map_err(|_| ProxyError::Deadline)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbound_headers_keep_only_valid_context_without_an_sdk_span() {
        let mut headers = HeaderMap::new();
        headers.append("traceparent", HeaderValue::from_static("invalid"));
        headers.append("tracestate", HeaderValue::from_static("untrusted=old"));
        headers.append("baggage", HeaderValue::from_static("document=private"));
        let mut trace = indra_telemetry::Carrier::from_parts(
            Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"),
            Some("vendor=opaque"),
        );
        forward_trace(&mut headers, &mut trace);
        assert_eq!(headers.get("traceparent").unwrap(), trace.parent().unwrap());
        assert_eq!(headers.get("tracestate").unwrap(), "vendor=opaque");
        assert!(!headers.contains_key("baggage"));
        let mut empty = indra_telemetry::Carrier::empty();
        forward_trace(&mut headers, &mut empty);
        assert!(!headers.contains_key("traceparent"));
        assert!(!headers.contains_key("tracestate"));
    }

    #[tokio::test]
    async fn ocr_probe_requires_complete_bounded_ready_json() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (status, content, expected) in [
            ("200 OK", "{\"phase\":\"ready\",\"active\":8}", true),
            ("200 OK", "{\"phase\":\"draining\",\"active\":0}", false),
            ("503 Unavailable", "{\"phase\":\"ready\"}", false),
            ("200 OK", "not JSON", false),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let peer = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut head = [0; 1024];
                let mut used = 0;
                while !head[..used].ends_with(b"\r\n\r\n") {
                    socket.read_exact(&mut head[used..used + 1]).await.unwrap();
                    used += 1;
                }
                assert!(head[..used].starts_with(b"GET /readyz HTTP/1.1\r\n"));
                socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{content}", content.len()).as_bytes()).await.unwrap();
            });
            assert_eq!(ocr_ready(address).await.unwrap_or(false), expected);
            peer.await.unwrap();
        }
    }

    #[tokio::test]
    async fn stalled_response_channel_detaches_but_drains_backend_to_eof() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        static SHARED: std::sync::OnceLock<Shared> = std::sync::OnceLock::new();
        static DATA: [u8; 128 * 1024] = [b'x'; 128 * 1024];
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let shared = SHARED.get_or_init(|| {
            let shared = Shared::new(crate::configuration::Gateway {
                backend: listener.local_addr().unwrap(),
                max_active: 1,
                max_request_bytes: 1024,
                max_response_bytes: DATA.len(),
                client_stall_seconds: 1,
                ..crate::configuration::Gateway::default()
            });
            shared.lifecycle.activate().unwrap();
            shared.backend_ready.store(true, Ordering::Release);
            shared
        });
        let peer = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut headers = [0; 1024];
            let mut used = 0;
            while !headers[..used].ends_with(b"\r\n\r\n") {
                socket
                    .read_exact(&mut headers[used..used + 1])
                    .await
                    .unwrap();
                used += 1;
            }
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 131072\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            socket.write_all(&DATA).await.unwrap();
        });
        let (front, backend, upload) = shared.admit().unwrap();
        let request = Request::builder()
            .uri("/tika")
            .header(header::HOST, "localhost")
            .header(header::CONNECTION, "close")
            .body(Full::new(Bytes::from_owner(upload)))
            .unwrap();
        let (sender, receiver) = oneshot::channel();
        let exchange = tokio::spawn(exchange(
            shared,
            request,
            sender,
            Instant::now() + Duration::from_secs(5),
        ));
        let head = receiver.await.unwrap().unwrap();
        // Keeping the receiver without polling it deterministically fills the
        // single channel slot, independent of OS socket-buffer sizes.
        assert!(
            tokio::time::timeout(Duration::from_secs(3), exchange)
                .await
                .unwrap()
                .unwrap()
                .is_ok()
        );
        peer.await.unwrap();
        drop(backend);
        assert_eq!(shared.lifecycle.snapshot().active, 1);
        shared.drain();
        assert!(shared.admit().is_none());
        drop(head);
        drop(front);
        assert_eq!(shared.lifecycle.snapshot().active, 0);
    }

    #[tokio::test]
    async fn final_transport_bytes_keep_frontend_admission_after_body_drop() {
        let shared = service::tests::shared(2);
        let (front, backend, upload) = shared.admit().unwrap();
        drop(backend);
        drop(upload);
        let (sender, receiver) = mpsc::channel(2);
        sender
            .send(StreamItem::Chunk(Bytes::from_static(b"retained")))
            .await
            .unwrap();
        sender.send(StreamItem::Complete).await.unwrap();
        drop(sender);
        let mut body = StreamBody {
            receiver,
            pending: None,
            finished: false,
            front: Some(front),
            outcome: "complete",
        };
        let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
        drop(body);
        assert_eq!(shared.lifecycle.snapshot().active, 1);
        let retained = bytes.slice(1..2);
        drop(bytes);
        assert_eq!(shared.lifecycle.snapshot().active, 1);
        drop(retained);
        assert_eq!(shared.lifecycle.snapshot().active, 0);
    }

    #[tokio::test]
    async fn missing_explicit_completion_is_a_body_error_not_successful_eof() {
        let shared = service::tests::shared(3);
        let (front, backend, upload) = shared.admit().unwrap();
        drop(backend);
        drop(upload);
        let (sender, receiver) = mpsc::channel(1);
        sender
            .send(StreamItem::Chunk(Bytes::from_static(b"partial")))
            .await
            .unwrap();
        drop(sender);
        let mut body = StreamBody {
            receiver,
            pending: None,
            finished: false,
            front: Some(front),
            outcome: "complete",
        };
        assert!(matches!(
            body.frame().await,
            Some(Err(ProxyError::Upstream))
        ));
        drop(body);
        assert_eq!(shared.lifecycle.snapshot().active, 0);
    }

    #[test]
    fn framing_is_bounded_and_connection_nominations_are_removed() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("4"));
        headers.insert(header::CONNECTION, HeaderValue::from_static("close, x-hop"));
        headers.insert("x-hop", HeaderValue::from_static("remove"));
        headers.insert("x-tika-ocrskipocr", HeaderValue::from_static("true"));
        assert_eq!(request_headers(&headers, &Method::PUT, 4).unwrap(), 4);
        strip_hop_headers(&mut headers).unwrap();
        assert!(!headers.contains_key("x-hop"));
        assert!(headers.contains_key("x-tika-ocrskipocr"));
        assert!(matches!(
            request_headers(&headers, &Method::PUT, 3),
            Err(ProxyError::TooLarge)
        ));
        headers.append(header::CONTENT_LENGTH, HeaderValue::from_static("4"));
        assert!(matches!(
            request_headers(&headers, &Method::PUT, 4),
            Err(ProxyError::Length)
        ));
        headers.remove(header::CONTENT_LENGTH);
        headers.insert(
            header::CONNECTION,
            HeaderValue::from_static("content-length"),
        );
        assert!(strip_hop_headers(&mut headers).is_err());
        headers.clear();
        headers.insert(
            header::TRANSFER_ENCODING,
            HeaderValue::from_static("chunked"),
        );
        assert!(request_headers(&headers, &Method::PUT, 4).is_err());
    }
}
