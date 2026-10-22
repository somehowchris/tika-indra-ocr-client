use super::state::{Shared, Work};
use crate::{
    error::{self, ErrorCode, ErrorResponse},
    protocol::{self, RequestId},
};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{ConnectInfo, Request, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use indra_core::lifecycle::{Phase, Snapshot};
use serde::Serialize;
use std::{
    net::SocketAddr,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
use tracing::Instrument;
use utoipa::{OpenApi, ToSchema};

pub(super) fn router(shared: &'static Shared) -> Router {
    Router::new()
        .route("/ocr", post(ocr))
        .route("/healthz", get(health))
        .route("/readyz", get(ready))
        .route("/startupz", get(startup))
        .route("/drain", post(drain))
        .route("/openapi.json", get(openapi))
        .with_state(shared)
}

#[derive(Serialize, ToSchema)]
pub(super) struct Status {
    /// starting, ready, draining, stopped or failed.
    phase: &'static str,
    /// Uploads, queued/running work and retained response bodies.
    active: u16,
    workers_ready: u8,
    model_revision: &'static str,
}

fn status(shared: &Shared, state: Snapshot, ready: bool) -> Response {
    let mut response = (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(Status {
            phase: state.phase.as_str(),
            active: state.active,
            workers_ready: shared.ready_workers.load(Ordering::Acquire),
            model_revision: indra_models::MODEL_REVISION,
        }),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    response
}

#[utoipa::path(get, path = "/healthz", tag = "Lifecycle", responses(
    (status = 200, description = "Process supervisor is healthy; includes startup and normal draining", body = Status),
    (status = 503, description = "Terminal worker/supervisor failure", body = Status)
))]
async fn health(State(shared): State<&'static Shared>) -> Response {
    let state = shared.lifecycle.snapshot();
    let healthy = !matches!(state.phase, Phase::Failed | Phase::Stopped);
    status(shared, state, healthy)
}

#[utoipa::path(get, path = "/readyz", tag = "Lifecycle", responses(
    (status = 200, description = "Every model worker is verified and warmed; admission is open", body = Status),
    (status = 503, description = "Starting, draining, stopped or failed", body = Status)
))]
async fn ready(State(shared): State<&'static Shared>) -> Response {
    // Temporary saturation is a per-request 503, not a readiness transition.
    let state = shared.lifecycle.snapshot();
    let ready = state.phase == Phase::Ready;
    status(shared, state, ready)
}

#[utoipa::path(get, path = "/startupz", tag = "Lifecycle", responses(
    (status = 200, description = "All workers completed model verification and warmup", body = Status),
    (status = 503, description = "Warmup has not completed or process has failed", body = Status)
))]
async fn startup(State(shared): State<&'static Shared>) -> Response {
    let state = shared.lifecycle.snapshot();
    let started = shared.warmed.load(Ordering::Acquire) && state.phase != Phase::Failed;
    status(shared, state, started)
}

#[utoipa::path(post, path = "/drain", tag = "Lifecycle", responses(
    (status = 200, description = "Loopback caller removed readiness; accepted work will finish", body = Status),
    (status = 403, description = "Drain requires a loopback peer; forwarded headers are ignored", body = ErrorResponse)
))]
async fn drain(State(shared): State<&'static Shared>, request: Request) -> Response {
    if !request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_some_and(|ConnectInfo(peer)| peer.ip().is_loopback())
    {
        return error::response(ErrorCode::Forbidden, &RequestId::next());
    }
    shared.drain("loopback_http");
    status(shared, shared.lifecycle.snapshot(), true)
}

struct ImageInput;

impl utoipa::PartialSchema for ImageInput {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::Schema> {
        use utoipa::openapi::schema::{KnownFormat, ObjectBuilder, SchemaFormat, Type};
        ObjectBuilder::new()
            .schema_type(Type::String)
            .format(Some(SchemaFormat::KnownFormat(KnownFormat::Binary)))
            .into()
    }
}

impl ToSchema for ImageInput {}

#[utoipa::path(post, path = "/ocr", tag = "OCR",
    request_body(description = "Encoded PNG, JPEG or TIFF bytes. One positive Content-Length is required. Upload, queue and execution share the request deadline.",
        content((ImageInput = "application/octet-stream"), (ImageInput = "image/png"), (ImageInput = "image/jpeg"), (ImageInput = "image/tiff"))),
    params(
        ("Content-Length" = usize, Header, description = "Positive encoded byte length; at most the configured input limit"),
        ("traceparent" = Option<String>, Header, description = "Optional W3C version-00 parent; invalid context is ignored"),
        ("tracestate" = Option<String>, Header, description = "Optional W3C trace state, at most 512 bytes; baggage is not propagated")
    ), responses(
        (status = 200, description = "UTF-8 text in Indra reading order, possibly empty", body = String, content_type = "text/plain; charset=utf-8", headers(("X-Request-ID" = String, description = "Generated 32-character correlation ID"))),
        (status = 400, description = "Invalid request framing, trailers or body length", body = ErrorResponse),
        (status = 411, description = "Content-Length missing or repeated", body = ErrorResponse),
        (status = 413, description = "Configured encoded/decoded/output bound exceeded", body = ErrorResponse),
        (status = 415, description = "Content type must be application/octet-stream, image/png, image/jpeg or image/tiff", body = ErrorResponse),
        (status = 422, description = "Invalid or unsupported encoded image", body = ErrorResponse),
        (status = 503, description = "Admission full or service not ready; Retry-After: 1", body = ErrorResponse),
        (status = 504, description = "Upload or request deadline expired", body = ErrorResponse),
        (status = 500, description = "Terminal native or internal ownership failure; details are redacted", body = ErrorResponse)
    )
)]
async fn ocr(State(shared): State<&'static Shared>, request: Request) -> Response {
    let request_id = RequestId::next();
    let span = {
        let trace = indra_telemetry::Carrier::from_headers(
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
        indra_telemetry::request_span(request_id.as_str(), trace.parent(), trace.state())
    };
    async move {
        let mut timing = RequestTiming {
            request_id,
            begin: Instant::now(),
            outcome: "cancelled",
        };
        match accepted(shared, request, request_id, timing.begin).await {
            Ok(response) => {
                timing.outcome = "ok";
                tracing::Span::current().record("http.response.status_code", 200);
                response
            }
            Err(code) => {
                timing.outcome = code.name();
                tracing::Span::current()
                    .record("http.response.status_code", code.status().as_u16());
                tracing::Span::current().record("error.type", code.name());
                error::response(code, &request_id)
            }
        }
    }
    .instrument(span)
    .await
}

async fn accepted(
    shared: &'static Shared,
    request: Request,
    request_id: RequestId,
    begin: Instant,
) -> Result<Response, ErrorCode> {
    let expected = protocol::expected_length(request.headers(), shared.settings.max_request_bytes)?;
    let mut reservation = shared.reserve()?;
    let deadline = begin + shared.settings.request_timeout();
    let upload_deadline = begin + Duration::from_secs(shared.settings.upload_timeout_seconds);
    tokio::time::timeout_at(
        upload_deadline.into(),
        protocol::read_body(
            request.into_body(),
            expected,
            &mut reservation.buffers_mut().input,
        ),
    )
    .await
    .map_err(|_| ErrorCode::Deadline)??;
    let (response, receiver) = oneshot::channel();
    let work = Work {
        reservation,
        response,
        request_id,
        deadline,
        queued_at: Instant::now(),
        span: tracing::info_span!(
            "ocr.execute",
            request_id = request_id.as_str(),
            model_revision = indra_models::MODEL_REVISION,
            worker = tracing::field::Empty,
            queue_wait_ms = tracing::field::Empty,
            outcome = tracing::field::Empty
        ),
    };
    shared.enqueue(work)?;
    let completed = tokio::time::timeout_at(deadline.into(), receiver)
        .await
        .map_err(|_| ErrorCode::Deadline)?
        .map_err(|_| ErrorCode::Internal)??;
    let length = completed.reservation.buffers().text.len();
    if completed.stats.output_bytes != length {
        shared.fail("output_byte_accounting");
        return Err(ErrorCode::Internal);
    }
    // bytes owns the reservation until the HTTP transport releases its final
    // reference. No second text buffer, payload copy or detached lease is used.
    let body = Body::from(Bytes::from_owner(completed.reservation));
    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    protocol::set_common_headers(headers, &request_id);
    Ok(response)
}

struct RequestTiming {
    request_id: RequestId,
    begin: Instant,
    outcome: &'static str,
}

impl Drop for RequestTiming {
    fn drop(&mut self) {
        let elapsed = self.begin.elapsed();
        indra_telemetry::request_finished(self.outcome, elapsed);
        tracing::info!(
            event = "http_request_finished",
            request_id = self.request_id.as_str(),
            outcome = self.outcome,
            elapsed_ms = elapsed.as_secs_f64() * 1000.0,
            "OCR HTTP handler finished"
        );
    }
}

#[utoipa::path(get, path = "/openapi.json", tag = "Documentation", responses(
    (status = 200, description = "OpenAPI 3.1 document generated from the running handlers")
))]
async fn openapi() -> Json<utoipa::openapi::OpenApi> {
    Json(document())
}

#[derive(OpenApi)]
#[openapi(info(title = "Indra CPU OCR service",
    description = "Bounded local CPU OCR for Apache Tika. OCR responses are UTF-8 text. Admission includes upload, queue, inference and retained response storage. Only validated traceparent/tracestate metadata is propagated. Images, OCR text, filenames, authorization headers and baggage are excluded from telemetry."),
    paths(ocr, health, ready, startup, drain, openapi),
    components(schemas(ImageInput, Status, ErrorCode, ErrorResponse)),
    tags((name = "OCR", description = "Bounded image recognition"), (name = "Lifecycle", description = "Model readiness and graceful shutdown"), (name = "Documentation", description = "Machine-readable API"))
)]
struct Api;

pub(super) fn document() -> utoipa::openapi::OpenApi {
    Api::openapi()
}
